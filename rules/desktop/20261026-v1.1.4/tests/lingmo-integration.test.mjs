import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync, writeFileSync, mkdtempSync, rmSync, existsSync, readdirSync, copyFileSync, cpSync, unlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {inflateSync} from 'node:zlib';
import {runPython} from '../scripts/python-runtime.mjs';

const base = fileURLToPath(new URL('..', import.meta.url));
const fixture = JSON.parse(readFileSync(join(base, 'tests/fixtures/lingmo-v7-bundle.json'), 'utf8'));
const cli = (args) => spawnSync(process.execPath, [join(base, 'scripts/storyboard.mjs'), ...args], {encoding:'utf8'});
const assetsCli = (args) => spawnSync(process.execPath, [join(base, 'scripts/assets.mjs'), ...args], {encoding:'utf8'});
const pngCli = (index, directory, verify=false) => runPython([join(base, 'scripts/make-white-placeholders.py'), '--index', index, '--output-dir', directory, ...(verify?['--verify']:[])], {encoding:'utf8'});
function sandbox(fn) {
  const directory = mkdtempSync(join(tmpdir(), 'lingmo-acceptance-'));
  try { return fn(directory); } finally { rmSync(directory, {recursive:true, force:true}); }
}
function save(directory, filename, value) {
  const file = join(directory, filename);
  writeFileSync(file, typeof value === 'string' ? value : JSON.stringify(value, null, 2));
  return file;
}
const fresh = () => structuredClone(fixture);
function ok(result) { assert.equal(result.status, 0, `${result.stderr}\n${result.stdout}`); return result; }
function exported(directory, bundle=fresh(), options=[]) {
  const input=save(directory, 'bundle.json', bundle), output=join(directory, 'final.md');
  ok(cli(['check', '--input', input, ...options.filter((_,i)=>options[i-1]==='--registry'||options[i]==='--registry')]));
  ok(cli(['export', '--input', input, '--output', output, ...options]));
  const explicit=options.indexOf('--assets-index');
  const index=explicit<0?`${output}.assets.json`:options[explicit+1];
  assert.equal(existsSync(index),true,'导出时必须同步生成素材目录');
  const catalog=JSON.parse(readFileSync(index,'utf8'));
  assert.equal(catalog.version,'lingmo-v7-assets-v1');
  assert.ok(Array.isArray(catalog.assets));
  ok(cli(['verify','--input',input,'--output',output,...options]));
  return {input,output,index,catalog,markdown:readFileSync(output,'utf8')};
}
function segments(markdown) {
  return markdown.split(/(?=^片段\d+\n时长：\d+秒$)/m).slice(1);
}
function tokens(markdown) { return [...markdown.matchAll(/@\[([^\]\n]+)\]/g)].map(m=>m[1]); }
function assertTokens(fragment, expected) {
  assert.deepEqual(tokens(fragment).sort(),expected.toSorted());
  for(const line of fragment.split('\n'))if(line.includes('@['))
    assert.match(line,/^\[(?:场景|本单元道具|本单元出场人物)\]/,'@ 只嵌在三个约定的标签行');
}
function addAsset(bundle, name, type, importName, status='常态') {
  const asset={key:name+type,name,type,status,importName,placeholderOrdinal:Math.max(-1,...bundle.assets.map(a=>a.placeholderOrdinal))+1};
  bundle.assets.push(asset);return asset;
}
function oneScene(bundle) {
  const e=bundle.episodes[0];bundle.scope[0].sceneIds=['1-1'];
  e.scenes=[{...e.scenes[0],sourceRefs:e.scenes.flatMap(s=>s.sourceRefs),bindings:[...new Set(e.scenes.flatMap(s=>s.bindings))]}];
  e.draft.units.forEach(u=>u.sceneId='1-1');
}
function pureVoice(bundle) {
  const e=bundle.episodes[0],u=e.draft.units[1],a=addAsset(bundle,'巡检员','CH','巡检员','仅远处传声');
  e.source.sourceRows[3].text='巡检员（VO）：门锁正常。';
  e.scenes[1].bindings.push(a.key);u.bindings.push(a.key);
  u.shots[0].dialogueParts[0].presentation='VO';u.voiceBindings=[{lineId:'E01:R4:D2',assetKey:a.key}];
  return a;
}
function bodyOnly(bundle) {
  const e=bundle.episodes[0],u=e.draft.units[0];
  e.source.sourceRows[0].text+='原文明示：值守者短暂以Q版小人表现犹豫。';
  u.visualSubjects=[{name:'值守者Q版小人',domain:'subjective'}];
  u.visualSubjectBindings=[{name:'值守者Q版小人',domain:'subjective',assetKey:null}];
  u.shots[0].picture+='主观层的值守者Q版小人短暂缩肩，随后回到现实原站位。';
}
function rejects(name, mutate, diagnostic= /\S/) {
  test(name,()=>sandbox(directory=>{
    const bundle=fresh();mutate(bundle);
    const result=cli(['check','--input',save(directory,'bundle.json',bundle)]);
    assert.notEqual(result.status,0,'不合法输入应被拒绝');assert.match(result.stderr,diagnostic);
  }));
}

test('v7 原入口导出正文内嵌格式并同步产出同名素材目录', () => sandbox(directory => {
  const input = save(directory, 'bundle.json', fixture), output = join(directory, 'final.md');
  const result = cli(['export', '--input', input, '--output', output]);
  assert.equal(result.status, 0, result.stderr);
  const markdown = readFileSync(output, 'utf8');
  assert.match(markdown, /\[通用提示词\]/);
  assert.doesNotMatch(markdown, /^引用：/m);
  assert.equal(existsSync(`${output}.assets.json`), true, '默认素材目录必须与 MD 一起写出');
}));

test('同一原场前段不带后段系统屏，场级三行仅为图片并集',()=>sandbox(directory=>{
  const b=fresh(),e=b.episodes[0],u=e.draft.units[1],screen=addAsset(b,'进化系统光幕','PR','进化系统光幕');
  e.source.sourceRows[2].text+='稍后进化系统光幕亮起。';u.props.push(screen.name);u.bindings.push(screen.key);u.imageBindings.push(screen.key);
  e.scenes[1].bindings.push(screen.key);u.shots[0].picture+='此时进化系统光幕亮起。';oneScene(b);
  const {markdown,catalog}=exported(directory,b),parts=segments(markdown);
  assert.equal(parts.length,2);assert.match(markdown,/^1-1\n场景：门厅、走廊\n人物：值守者面罩\n道具：铜钥匙、进化系统光幕$/m);
  assertTokens(parts[0],['门厅SC','值守者面罩CH','铜钥匙PR']);
  assertTokens(parts[1],['走廊SC','进化系统光幕PR']);
  assert.equal(catalog.assets.length,5);
  assert.equal((markdown.match(/^\*\*【标准化剧本】\*\*$/gm)||[]).length,1);
  assert.equal((markdown.match(/^场景：/gm)||[]).length,1);
  assert.doesNotMatch(markdown,/^引用：|^片段正文：|^\[主观视觉主体\]|^(?:日|夜) (?:内|外)$/m);
}));

test('纯 VO 保留声源身份与原声，三行和素材目录不自动增加说者图片',()=>sandbox(directory=>{
  const b=fresh(),voice=pureVoice(b),before=structuredClone(b),{markdown,catalog}=exported(directory,b),parts=segments(markdown);
  assertTokens(parts[1],['走廊SC']);assert.match(parts[1],/巡检员[\s\S]*VO/);assert.match(parts[1],/门锁正常。/);
  assert.match(markdown,/^1-2\n场景：走廊\n人物：无\n道具：无$/m);
  assert.equal(catalog.assets.some(a=>a.token===`${voice.importName}CH`),false);
  assert.deepEqual(b,before,'导出不得改原人声映射或在场性');
}));
rejects('纯 VO 不能仅凭发声身份加入 imageBindings',b=>{const a=pureVoice(b);b.episodes[0].draft.units[1].imageBindings.push(a.key);},/U2|1-2|图片|IMAGE/);

test('屏幕人物保留媒体来源并可引用图片，不升级为现实出场',()=>sandbox(directory=>{
  const b=fresh(),e=b.episodes[0],u=e.draft.units[1],a=addAsset(b,'录像中值守者','CH','录像中值守者','录像人物');
  e.source.sourceRows[2].text+='墙上监控屏显示戴面罩的值守者录像。';
  u.mediaSubjects=[{name:a.name,medium:'screen',sourceRefs:['E01:R3']}];u.bindings.push(a.key);u.imageBindings.push(a.key);e.scenes[1].bindings.push(a.key);
  u.shots[0].picture+='监控屏内显示戴面罩的值守者录像；屏外走廊没有现实人物。';
  const {markdown}=exported(directory,b),parts=segments(markdown);
  assertTokens(parts[1],['走廊SC','录像中值守者CH']);assert.match(parts[1],/监控屏内[\s\S]*屏外走廊没有现实人物/);
  assert.deepEqual(u.participants,[]);assert.deepEqual(u.visualSubjects,[]);
}));

test('源明示 Q 版 body_only 不造卡且保留原生 visualSubjects 和正文',()=>sandbox(directory=>{
  const b=fresh();bodyOnly(b);const before=structuredClone(b),{markdown,catalog}=exported(directory,b);
  assert.match(markdown,/主观层的值守者Q版小人短暂缩肩/);assert.doesNotMatch(markdown,/^\[主观视觉主体\]/m);
  assert.equal(catalog.assets.length,fixture.assets.length);assertTokens(segments(markdown)[0],['门厅SC','值守者面罩CH','铜钥匙PR']);
  assert.deepEqual(b,before);
}));
rejects('Q 版有主体却漏明确图片或正文映射时拒绝',b=>{bodyOnly(b);delete b.episodes[0].draft.units[0].visualSubjectBindings;},/VISUAL|主体|U1/);
rejects('视觉映射域不得偷换原生主体域',b=>{bodyOnly(b);b.episodes[0].draft.units[0].visualSubjectBindings[0].domain='prospective';},/VISUAL|主体|U1/);
rejects('视觉映射不能额外虚造一个未声明主体',b=>{bodyOnly(b);b.episodes[0].draft.units[0].visualSubjectBindings.push({name:'不存在的幻影',domain:'subjective',assetKey:null});},/VISUAL|主体|U1/);

test('同一角色持续状态分别引用，后段受伤状态不提前带入前段',()=>sandbox(directory=>{
  const b=fresh(),e=b.episodes[0],u=e.draft.units[1],a=addAsset(b,'值守者·受伤','CH','值守者受伤','面罩仍遮口鼻，额头擦伤');
  e.source.sourceRows[2].text+='稍后额头擦伤的值守者进入走廊。';u.participants.push(a.name);u.bindings.push(a.key);u.imageBindings.push(a.key);e.scenes[1].bindings.push(a.key);
  u.shots[0].picture='额头擦伤的值守者在走廊停步，面罩仍遮住口鼻。';
  const {markdown,catalog}=exported(directory,b),parts=segments(markdown);
  assertTokens(parts[0],['门厅SC','值守者面罩CH','铜钥匙PR']);assertTokens(parts[1],['走廊SC','值守者受伤CH']);
  assert.equal(catalog.assets.filter(a=>a.type==='CH').length,2);
}));

test('29.1 秒及3.2秒只向上取整容器，内部镜长与声窗不补演',()=>sandbox(directory=>{
  const b=fresh(),[first,last]=b.episodes[0].draft.units;first.shots[0].duration=20;first.shots[1].duration=9.1;last.shots[0].duration=3.2;
  const {markdown}=exported(directory,b),parts=segments(markdown);
  assert.match(parts[0],/^片段1\n时长：30秒/m);assert.match(parts[0],/分镜1｜0-20s｜20s/);assert.match(parts[0],/分镜2｜20-29\.1s｜9\.1s/);
  assert.match(parts[1],/时长：4秒/);assert.match(parts[1],/分镜1｜0-3\.2s｜3\.2s/);
  assert.match(parts[0],/20[-–]23\.2秒/);assert.match(parts[1],/0\.2[-–]2\.8秒/);
  assert.equal((markdown.match(/^分镜\d+｜/gm)||[]).length,3);
}));
test('精确30秒不因浮点取整误成31秒',()=>sandbox(directory=>{
  const b=fresh();b.episodes[0].draft.units[0].shots[1].duration=27.5;
  const {markdown}=exported(directory,b);assert.match(segments(markdown)[0],/^片段1\n时长：30秒/m);assert.match(markdown,/分镜2｜2\.5-30s｜27\.5s/);
}));
test('镜窗和声窗1ns精度完整保留',()=>sandbox(directory=>{
  const b=fresh(),shots=b.episodes[0].draft.units[0].shots;shots[0].duration=2.500000001;
  shots[0].dialogueParts[0].voiceStart=1e-9;shots[0].dialogueParts[0].voiceEnd=2e-9;
  const {markdown}=exported(directory,b);assert.match(markdown,/时长：7秒/);assert.match(markdown,/分镜2｜2\.500000001-6\.000000001s｜3\.5s/);
  assert.match(markdown,/0\.000000001[-–]0\.000000002秒/);
}));
rejects('30.000000001秒拒绝而非截断或抹平',b=>b.episodes[0].draft.units[0].shots[1].duration=27.500000001,/U1|片长|UNIT|DURATION/);
rejects('项目15秒上限仍按纳秒检查实际时间线',b=>{b.episodes[0].settings.maxUnitSeconds=15;b.episodes[0].draft.units[0].shots[1].duration=12.500000001;},/U1|15|UNIT|DURATION/);
rejects('超过9位非零声窗精度拒绝',b=>b.episodes[0].draft.units[0].shots[0].dialogueParts[0].voiceStart=1e-10,/TIME_PRECISION|精度/);

test('全部逐镜字段、源提示、跨镜原句、分段演出与结尾禁止项保真',()=>sandbox(directory=>{
  const b=fresh(),e=b.episodes[0],u=e.draft.units[0];e.unitDirectionVersion='unit-direction-v1';
  e.source.sourceRows[1].text='值守者（压低声音）：门还没开，先别摘下面罩。';
  u.unitDirection='整段导演指令：眼神落门后才提醒，手部节奏始终克制。';
  u.exitState='单元最终状态：右手仍持铜钥匙，面罩完整遮口鼻。';
  u.segments=u.shots.map((s,i)=>({title:`分段${i+1}标题`,unitDirection:`分段${i+1}演出：先压住肩再转眼。`,lighting:`分段${i+1}光影：冷白灯侧缘可见。`,initialState:`分段${i+1}入口：手指贴合铜钥匙。`,exitState:`分段${i+1}出口：视线停在关闭门上。`,shotIds:[s.id]}));
  for(const [i,s]of u.shots.entries())for(const field of ['camera','picture','lighting','sound','transition'])s[field]+=`\n保留${field}第${i+1}镜的第二行。`;
  const {markdown}=exported(directory,b),unindented=markdown.replace(/^  /gm,'');
  for(const unit of e.draft.units){
    for(const field of ['unitDirection','lighting','initialState','exitState'])assert.ok(unindented.includes(unit[field]),`丢失单元 ${unit.id}.${field}`);
    for(const segment of unit.segments||[])for(const field of ['title','unitDirection','lighting','initialState','exitState'])assert.ok(unindented.includes(segment[field]),`丢失分段 ${segment.title}.${field}`);
    for(const shot of unit.shots)for(const field of ['camera','picture','lighting','sound','transition'])assert.ok(unindented.includes(shot[field]),`丢失 ${shot.id}.${field}`);
  }
  assert.ok(unindented.includes(e.settings.visualStyle));assert.ok(unindented.includes(e.settings.prohibitions));
  assert.match(markdown,/压低声音/);assert.match(markdown,/口语/);assert.match(markdown,/压低音量，字音清楚/);
  for(const quote of ['门还没开，','先别摘下面罩。','门锁正常。'])assert.equal(markdown.split(quote).length-1,1,`原句片段必须只发声一次：${quote}`);
  for(const block of markdown.split(/^分镜\d+｜.*$/m).slice(1)){
    const positions=['机位/运镜：','画面：','光影：','声音：','台词：','转场：'].map(label=>block.indexOf(label));
    assert.ok(positions.every(p=>p>=0));assert.deepEqual(positions,positions.toSorted((a,b)=>a-b),'沿用 v7 已验证逐镜字段顺序');
  }
}));
for(const [cue,quote,parts,category]of [['画外咆哮','吼！',['吼','！'],'兽吼'],['尖叫','啊！',['啊','！'],'尖叫'],['低哼','哼！',['哼','！'],'非词人声']])
test(`v7 不吞掉非词人声：${category}`,()=>sandbox(directory=>{
  const b=fresh(),e=b.episodes[0];e.source.sourceRows[1].text=`值守者（${cue}）：${quote}`;
  e.draft.units[0].shots.forEach((shot,i)=>{shot.dialogueParts[0].text=parts[i];shot.dialogueParts[0].presentation='画外';});
  const {markdown}=exported(directory,b);assert.ok(markdown.includes(cue));assert.ok(markdown.includes(category));
  for(const text of parts)assert.ok(markdown.includes(`：${text}`));
}));

test('真实标准源接入的多行对白和句中表演括注完整进入v7正文',()=>sandbox(directory=>{
  const text='第1集\n1-1 夜 内 门厅\n人物：值守者\n△值守者戴面罩，右手握铜钥匙。\n值守者（VO）：\n“门还没开，\n先别摘下面罩。”\n值守者：我（压低声音）听见了。\n';
  const sourceInput=save(directory,'source.txt',text),known=save(directory,'speakers.json',['值守者']),sourceOutput=join(directory,'standard-source.json');
  ok(spawnSync(process.execPath,[join(base,'scripts/prepare-source.mjs'),'--text',sourceInput,'--key','E01','--known-speakers',known,'--output',sourceOutput],{encoding:'utf8'}));
  const source=JSON.parse(readFileSync(sourceOutput,'utf8'));assert.equal(source.dialogue.length,2);
  assert.equal(source.dialogue[0].quote,'“门还没开，\n先别摘下面罩。”');assert.equal(source.dialogue[1].performanceCues[0].text,'压低声音');
  const b=fresh(),e=b.episodes[0],u=e.draft.units[0];b.scope[0].sceneIds=['1-1'];e.source=source;e.scenes=e.scenes.slice(0,1);e.scenes[0].sourceRefs=source.sourceRows.map(row=>row.id);e.draft.units=[u];
  u.voiceBindings=source.dialogue.map(q=>({lineId:q.id,assetKey:b.assets[0].key}));
  u.shots.forEach((shot,i)=>{const q=source.dialogue[i];shot.duration=4;shot.sourceRefs=i?q.sourceIds:source.sourceRows.filter(row=>!source.dialogue[1].sourceIds.includes(row.id)).map(row=>row.id);shot.dialogueParts=[{lineId:q.id,text:q.quote,voiceStart:0.2,voiceEnd:3.8,presentation:i?'画内':'VO',delivery:'平稳说出原句'}];});
  const {markdown}=exported(directory,b),unindented=markdown.replace(/^  /gm,'');
  for(const q of source.dialogue)assert.ok(unindented.includes(q.quote),'多行原话不得丢行或改字');
  assert.ok(markdown.includes('压低声音'),'句中表演括注必须保留，不能被原声净文本吞掉');
  const split=structuredClone(b),splitUnit=split.episodes[0].draft.units[0],second=splitUnit.shots[1],third=structuredClone(second);
  second.duration=2;second.dialogueParts[0].text='我';second.dialogueParts[0].voiceEnd=1.8;
  third.id='S3';third.duration=2;third.dialogueParts[0].text='听见了。';third.dialogueParts[0].voiceEnd=1.8;splitUnit.shots.push(third);
  const splitResult=exported(mkdtempSync(join(directory,'split-')),split);
  assert.equal((splitResult.markdown.match(/压低声音/g)||[]).length,1,'同句跨镜拆分不能把完整括注复制到每个镜头');
}));

test('含竖线顿号括号的内部名称保留，三行、令牌与PNG文件名共用安全映射',()=>sandbox(directory=>{
  const b=fresh(),e=b.episodes[0],a=b.assets[0],oldKey=a.key,oldName=a.name;
  a.name='值守者（面罩）｜默认、夜班';a.key=a.name+'CH';a.importName='值守者面罩默认夜班';
  for(const scene of e.scenes)scene.bindings=scene.bindings.map(k=>k===oldKey?a.key:k);
  for(const unit of e.draft.units){unit.bindings=unit.bindings.map(k=>k===oldKey?a.key:k);unit.imageBindings=unit.imageBindings.map(k=>k===oldKey?a.key:k);unit.participants=unit.participants.map(n=>n===oldName?a.name:n);unit.voiceBindings.forEach(v=>{if(v.assetKey===oldKey)v.assetKey=a.key;});}
  const {markdown,catalog}=exported(directory,b),row=catalog.assets.find(row=>row.type==='CH');
  assert.match(markdown,/^人物：值守者面罩默认夜班$/m);assert.match(markdown,/@\[值守者面罩默认夜班CH\]/);
  assert.equal(row.name,a.importName);assert.equal(row.token,'值守者面罩默认夜班CH');assert.equal(row.filename,'角色-值守者面罩默认夜班.png');
  assert.equal(a.name,'值守者（面罩）｜默认、夜班');assert.equal(a.status,fixture.assets[0].status);
}));
rejects('同类型安全导入名碰撞必须报错，不合并状态',b=>addAsset(b,'铜-钥匙','PR','铜钥匙','另一把钥匙'),/IMPORT|COLLISION|重名|冲突|重复|安全名/);
rejects('importName 本身带结构分隔符时不能放行',b=>b.assets[0].importName='值守者、面罩',/IMPORT|安全|ASSET/);
rejects('白图持久编号重复不能放行',b=>b.assets[1].placeholderOrdinal=b.assets[0].placeholderOrdinal,/ORDINAL|编号|重复|ASSET/);
rejects('超过6000边长范围的白图编号必须拒绝',b=>b.assets[0].placeholderOrdinal=5701,/ORDINAL|编号|范围|ASSET/);
rejects('v7 每段 imageBindings 必填',b=>delete b.episodes[0].draft.units[0].imageBindings,/IMAGE|图片|U1/);
rejects('v7 图片绑定不得重复',b=>b.episodes[0].draft.units[0].imageBindings.push('门厅SC'),/IMAGE|图片|U1/);
rejects('v7 图片绑定必须属于本单元逻辑绑定',b=>b.episodes[0].draft.units[0].imageBindings.push('走廊SC'),/IMAGE|图片|U1/);

test('超过30个逻辑声源不占用图片名额',()=>sandbox(directory=>{
  const b=fresh(),e=b.episodes[0],u=e.draft.units[1],shot=u.shots[0];shot.duration=30;
  for(let i=0;i<31;i++){
    const name=`广播声者${i+1}`,a=addAsset(b,name,'CH',name,'纯画外声源'),ref=`E01:V${i+1}`,lineId=`${ref}:D${i+3}`;
    e.source.sourceRows.push({id:ref,text:`${name}（VO）：好。`});e.scenes[1].sourceRefs.push(ref);e.scenes[1].bindings.push(a.key);
    u.bindings.push(a.key);u.voiceBindings.push({lineId,assetKey:a.key});shot.sourceRefs.push(ref);
    shot.dialogueParts.push({lineId,text:'好。',voiceStart:3+i*0.5,voiceEnd:3.4+i*0.5,presentation:'VO',delivery:'逐人短促确认'});
  }
  const {markdown,catalog}=exported(directory,b);assertTokens(segments(markdown)[1],['走廊SC']);assert.equal(catalog.assets.length,4);
}));
function addManyImages(b,count) {
  const e=b.episodes[0],u=e.draft.units[0];
  for(let i=0;i<count;i++){const a=addAsset(b,`架上道具${i+1}`,'PR',`架上道具${i+1}`);u.props.push(a.name);u.bindings.push(a.key);u.imageBindings.push(a.key);e.scenes[0].bindings.push(a.key);}
}
test('每段30张实际图片允许，31张拒绝',()=>sandbox(directory=>{
  const b=fresh();addManyImages(b,27);ok(cli(['check','--input',save(directory,'thirty.json',b)]));
  addManyImages(b,1);const last=b.assets.at(-1);last.name='额外道具';last.key='额外道具PR';last.importName='额外道具';
  const e=b.episodes[0],u=e.draft.units[0];u.props[u.props.length-1]=last.name;u.bindings[u.bindings.length-1]=last.key;u.imageBindings[u.imageBindings.length-1]=last.key;e.scenes[0].bindings[e.scenes[0].bindings.length-1]=last.key;
  const result=cli(['check','--input',save(directory,'thirty-one.json',b)]);assert.notEqual(result.status,0);assert.match(result.stderr,/30|IMAGE|图片/);
}));

for(const sceneId of ['[1-1]','第2集','片段2','@场、无','内容：原场','0-1秒：','  原场'])
test(`特殊原场号逐字保留且镜头全部回读：${sceneId}`,()=>sandbox(directory=>{
  const b=fresh(),e=b.episodes[0];b.scope[0].sceneIds[1]=sceneId;e.scenes[1].id=sceneId;e.draft.units[1].sceneId=sceneId;
  const {markdown}=exported(directory,b);assert.ok(markdown.includes(`\n${sceneId}\n场景：`));assert.equal((markdown.match(/^分镜\d+｜/gm)||[]).length,3);
}));

for(const [name,mutate]of [
  ['正文改字',s=>s.replace('钥匙微动即停','钥匙抛到门外')],
  ['引用错绑',s=>s.replace('@[门厅SC]','@[走廊SC]')],
  ['镜窗改动',s=>s.replace('分镜1｜0-2.5s｜2.5s','分镜1｜0-2.6s｜2.6s')],
  ['漏一镜',s=>s.replace(/分镜2｜2\.5-6s｜3\.5s\n[\s\S]*?(?=\n1-2\n)/,'')]
])test(`v7 独立回读发现${name}并给出坐标`,()=>sandbox(directory=>{
  const {input,output,markdown}=exported(directory),changed=mutate(markdown);assert.notEqual(changed,markdown,'篡改必须实际命中');writeFileSync(output,changed);
  const result=cli(['verify','--input',input,'--output',output]);assert.notEqual(result.status,0);assert.match(result.stderr,/VERIFY/);assert.match(result.stderr,/1-1|U1|片段1|分镜[12]/,'回读问题应给出内容坐标');
}));
for(const [name,change]of [
  ['目录稳定ID错绑',c=>{[c.assets[0].id,c.assets[1].id]=[c.assets[1].id,c.assets[0].id];}],
  ['目录资产版本错绑',c=>c.assets[0].assetRevision+=1],
  ['目录令牌错绑',c=>c.assets[0].token='@[不存在的素材CH]'],
  ['目录文件名错绑',c=>c.assets[0].filename='角色-别的名字.png'],
  ['目录编号错绑',c=>c.assets[0].placeholderOrdinal=100],
  ['目录漏项',c=>c.assets.pop()],
  ['目录重复项',c=>c.assets.push(structuredClone(c.assets[0]))]
])test(`v7 verify 同时检查${name}`,()=>sandbox(directory=>{
  const {input,output,index,catalog}=exported(directory);change(catalog);writeFileSync(index,JSON.stringify(catalog));
  const result=cli(['verify','--input',input,'--output',output]);assert.notEqual(result.status,0);assert.match(result.stderr,/VERIFY|CATALOG|目录|素材/);
}));
test('自定 --assets-index 路径参与 export 和 verify，缺目录也拒绝',()=>sandbox(directory=>{
  const index=join(directory,'素材目录.json'),{input,output}=exported(directory,fresh(),['--assets-index',index]);
  assert.equal(existsSync(`${output}.assets.json`),false);unlinkSync(index);
  const result=cli(['verify','--input',input,'--output',output,'--assets-index',index]);assert.notEqual(result.status,0);
}));

// Independent PNG decoder: validates framing/CRC, inflates IDAT and reverses all PNG filters.
// It does not import or execute the placeholder writer to decide whether pixels are white.
function crc32(bytes) { let crc=0xffffffff;for(const byte of bytes){crc^=byte;for(let i=0;i<8;i++)crc=(crc>>>1)^((crc&1)?0xedb88320:0);}return (crc^0xffffffff)>>>0; }
function decodePng(bytes) {
  assert.deepEqual(bytes.subarray(0,8),Buffer.from([137,80,78,71,13,10,26,10]));
  let cursor=8,header,ended=false;const idat=[];
  while(cursor<bytes.length){
    const length=bytes.readUInt32BE(cursor),type=bytes.toString('ascii',cursor+4,cursor+8),data=bytes.subarray(cursor+8,cursor+8+length);
    assert.equal(data.length,length);assert.equal(bytes.readUInt32BE(cursor+8+length),crc32(bytes.subarray(cursor+4,cursor+8+length)),'PNG CRC');
    if(type==='IHDR'){assert.equal(cursor,8);assert.equal(length,13);header=data;}
    if(type==='IDAT')idat.push(data);
    cursor+=12+length;if(type==='IEND'){ended=true;break;}
  }
  assert.ok(header&&ended&&idat.length);assert.equal(cursor,bytes.length);
  const width=header.readUInt32BE(0),height=header.readUInt32BE(4),depth=header[8],color=header[9],channels={0:1,2:3,4:2,6:4}[color];
  assert.equal(depth,8);assert.ok(channels);assert.equal(header[10],0);assert.equal(header[11],0);assert.equal(header[12],0);
  const raw=inflateSync(Buffer.concat(idat)),stride=width*channels;assert.equal(raw.length,(stride+1)*height);
  let previous=Buffer.alloc(stride);
  const paeth=(a,b,c)=>{const p=a+b-c,pa=Math.abs(p-a),pb=Math.abs(p-b),pc=Math.abs(p-c);return pa<=pb&&pa<=pc?a:pb<=pc?b:c;};
  for(let row=0;row<height;row++){
    const filter=raw[row*(stride+1)],scan=Buffer.alloc(stride);assert.ok(filter>=0&&filter<=4);
    for(let i=0;i<stride;i++){
      const left=i<channels?0:scan[i-channels],up=previous[i],corner=i<channels?0:previous[i-channels];
      const prediction=[0,left,up,Math.floor((left+up)/2),paeth(left,up,corner)][filter];scan[i]=(raw[row*(stride+1)+1+i]+prediction)&255;
    }
    assert.ok(scan.every(byte=>byte===255),'像素须为不透明纯白');previous=scan;
  }
  return {width,height};
}
const md5=bytes=>createHash('md5').update(bytes).digest('hex');
function addTextChunk(bytes,text) {
  const data=Buffer.from(`test\0${text}`),type=Buffer.from('tEXt'),chunk=Buffer.alloc(data.length+12);
  chunk.writeUInt32BE(data.length);type.copy(chunk,4);data.copy(chunk,8);chunk.writeUInt32BE(crc32(Buffer.concat([type,data])),8+data.length);
  return Buffer.concat([bytes.subarray(0,-12),chunk,bytes.subarray(-12)]);
}
test('白图是独立可解码的纯白PNG，名称数量尺寸MD5与导出目录一致',()=>sandbox(directory=>{
  const {index,catalog}=exported(directory),images=join(directory,'images');ok(pngCli(index,images));ok(pngCli(index,images,true));
  assert.deepEqual(readdirSync(images).sort(),catalog.assets.map(a=>a.filename).sort());
  const fingerprints=[];
  for(const asset of catalog.assets){const bytes=readFileSync(join(images,asset.filename)),decoded=decodePng(bytes);assert.deepEqual(decoded,{width:300+asset.placeholderOrdinal,height:300+asset.placeholderOrdinal});fingerprints.push(md5(bytes));}
  assert.equal(new Set(fingerprints).size,catalog.assets.length);
}));
test('素材目录重排和新增项不改变既有白图编号或内容',()=>sandbox(directory=>{
  const {index,catalog}=exported(directory),first=join(directory,'first'),second=join(directory,'second');ok(pngCli(index,first));
  const extended=structuredClone(catalog);extended.assets.reverse();extended.assets.push({id:'EXTRA',assetRevision:1,name:'备用道具',type:'PR',token:'备用道具PR',filename:'道具-备用道具.png',placeholderOrdinal:20});
  const extendedFile=save(directory,'extended.json',extended);ok(pngCli(extendedFile,second));
  for(const asset of catalog.assets)assert.deepEqual(readFileSync(join(second,asset.filename)),readFileSync(join(first,asset.filename)));
  assert.deepEqual(decodePng(readFileSync(join(second,'道具-备用道具.png'))),{width:320,height:320});
}));
for(const [name,mutate]of [
  ['缺图',(folder,assets)=>unlinkSync(join(folder,assets[0].filename))],
  ['多图',(folder,assets)=>copyFileSync(join(folder,assets[0].filename),join(folder,'道具-多余.png'))],
  ['复制同图',(folder,assets)=>copyFileSync(join(folder,assets[0].filename),join(folder,assets[1].filename))],
  ['同尺寸但不同MD5',(folder,assets)=>writeFileSync(join(folder,assets[1].filename),addTextChunk(readFileSync(join(folder,assets[0].filename)),'different-md5'))],
  ['损坏数据',(folder,assets)=>{const file=join(folder,assets[0].filename),bytes=readFileSync(file);bytes[bytes.length-16]^=1;writeFileSync(file,bytes);}]
])test(`白图 verify 发现${name}`,()=>sandbox(directory=>{
  const {index,catalog}=exported(directory),images=join(directory,'images');ok(pngCli(index,images));mutate(images,catalog.assets);
  const result=pngCli(index,images,true);assert.notEqual(result.status,0,'校验应拒绝错误图片包');assert.match(result.stderr,/\S/);
}));

test('生产总库 init→补直播手机→消费回执→v7及图片目录闭环',()=>sandbox(directory=>{
  const b=fresh(),registryDirectory=join(directory,'registry'),sourceHash='f'.repeat(64);b.testFixture=false;b.sourceHash=sourceHash;
  const seed={importProfile:'lingmo-v7',sourceHash,assets:b.assets.map(a=>({...a,aliases:[],origin:{kind:'source',sourceRefs:['E01:R1'],basis:'合成源中明确的身份、场景与持物。'},design:{state:'TEXT_ADOPTED',text:'已采用的文字卡保持合成源特征。'}}))};
  const initialized=JSON.parse(ok(assetsCli(['init','--registry',registryDirectory,'--input',save(directory,'seed.json',seed)])).stdout);
  b.assetRegistry={id:initialized.id,revision:initialized.revision};
  let registry=JSON.parse(ok(assetsCli(['status','--registry',registryDirectory])).stdout).registry;
  b.assets=registry.assets.map(({id,revision,...a})=>({...a,assetId:id,assetRevision:revision}));
  const oldSnapshot=readFileSync(join(registryDirectory,'v000001.json'));
  const phone={name:'直播手机（竖屏）',key:'直播手机（竖屏）PR',type:'PR',status:'竖屏直播',aliases:['手持直播设备'],origin:{kind:'design',sourceRefs:['E01:R1'],basis:'以直播功能需求选用同一手机设备。'},design:{state:'TEXT_ADOPTED',text:'黑色直板手机，竖向持握，背部双摄。'}};
  const change={version:'series-asset-change-v1',baseRevision:1,requestId:'ACCEPTANCE-PHONE',action:'add',reason:'本集生产需要直播设备',duplicateReview:'逐一核对现有道具无手机',uses:[{episode:1,sceneId:'1-1',unitId:'U1'}],asset:phone};
  ok(assetsCli(['apply','--registry',registryDirectory,'--input',save(directory,'change.json',change)]));
  assert.deepEqual(readFileSync(join(registryDirectory,'v000001.json')),oldSnapshot,'已有总库快照不可原地改写');
  const missing=cli(['check','--input',save(directory,'pending.json',b),'--registry',registryDirectory]);assert.notEqual(missing.status,0);assert.match(missing.stderr,/UNCONSUMED|消费/);
  registry=JSON.parse(ok(assetsCli(['status','--registry',registryDirectory])).stdout).registry;
  b.assets=registry.assets.map(({id,revision,...a})=>({...a,assetId:id,assetRevision:revision}));b.assetRegistry.revision=registry.revision;
  const currentPhone=b.assets.find(a=>a.key===phone.key),e=b.episodes[0],u=e.draft.units[0];
  u.props.push(phone.name);u.bindings.push(phone.key);u.imageBindings.push(phone.key);e.scenes[0].bindings.push(phone.key);u.shots[0].picture+='左手竖持直播手机，拍摄关闭的门。';
  const input=save(directory,'consumed.json',b),receipt=join(directory,'receipt.json');ok(assetsCli(['sync','--registry',registryDirectory,'--input',input,'--output',receipt]));
  assert.deepEqual(JSON.parse(readFileSync(receipt,'utf8')).closedRequests,['ACCEPTANCE-PHONE']);
  const {index,catalog,markdown}=exported(directory,b,['--registry',registryDirectory]);
  const delivered=catalog.assets.find(a=>a.id===currentPhone.assetId);assert.ok(delivered);assert.equal(delivered.assetRevision,1);assert.equal(delivered.name,'直播手机竖屏');assert.equal(delivered.token,'直播手机竖屏PR');assert.equal(delivered.placeholderOrdinal,4);
  assert.match(markdown,/@\[直播手机竖屏PR\]/);assert.doesNotMatch(markdown,/ACCEPTANCE-PHONE|assetRevision|sourceHash/);
  const images=join(directory,'images');ok(pngCli(index,images));ok(pngCli(index,images,true));
}));

test('旧稿四命令仍可用，新包复制到独立目录可导出和回读v7',()=>sandbox(directory=>{
  const legacy=JSON.parse(readFileSync(join(base,'tests/fixtures/export-bundle.json'),'utf8')),source=save(directory,'source.json',legacy.episodes[0].source);
  const inspected=JSON.parse(ok(cli(['inspect-source','--input',source])).stdout);assert.deepEqual(inspected.dialogue.map(q=>q.id),['E01:R2:D1','E01:R4:D2']);
  const legacyInput=save(directory,'legacy.json',legacy),legacyOutput=join(directory,'legacy.md');
  for(const command of ['check','export','verify'])ok(cli([command,'--input',legacyInput,...(command==='check'?[]:['--output',legacyOutput])]));
  assert.match(readFileSync(legacyOutput,'utf8'),/^引用：/m);
  const portable=join(directory,'portable');cpSync(join(base,'scripts'),join(portable,'scripts'),{recursive:true});
  const input=save(directory,'portable-input.json',fresh()),output=join(directory,'portable.md');
  for(const command of ['export','verify'])ok(spawnSync(process.execPath,[join(portable,'scripts/storyboard.mjs'),command,'--input',input,'--output',output],{encoding:'utf8',cwd:directory}));
  assert.equal(existsSync(`${output}.assets.json`),true);
}));
