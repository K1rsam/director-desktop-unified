import test from 'node:test';
import {checkBundle,projectBundle,renderProjection,verifyMarkdown} from '../scripts/storyboard.mjs';
import {normalizeSource} from '../scripts/core/runtime/storyboard-v4/core/source.mjs';
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,mkdtempSync,rmSync,cpSync,symlinkSync,existsSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {prepareSource} from '../scripts/prepare-source.mjs';
const base=fileURLToPath(new URL('..',import.meta.url)), fixture=JSON.parse(readFileSync(join(base,'tests/fixtures/export-bundle.json'),'utf8'));
const cli=(args)=>spawnSync(process.execPath,[join(base,'scripts/storyboard.mjs'),...args],{encoding:'utf8'});
function sandbox(fn){const dir=mkdtempSync(join(tmpdir(),'storyboard-test-'));try{return fn(dir);}finally{rmSync(dir,{recursive:true,force:true});}}
function input(dir,b=fixture){const file=join(dir,'bundle.json');writeFileSync(file,JSON.stringify(b));return file;}
function bad(name,mutate,code){test(name,()=>sandbox(dir=>{const b=structuredClone(fixture);mutate(b);const r=cli(['check','--input',input(dir,b)]);assert.notEqual(r.status,0);assert.match(r.stderr,new RegExp(code));}));}
test('完整合成输入可校验、导出和独立回读，跨镜原句只发声一次且保留小数',()=>sandbox(dir=>{const src=input(dir),out=join(dir,'final.md');let r=cli(['check','--input',src]);assert.equal(r.status,0,r.stderr);r=cli(['export','--input',src,'--output',out]);assert.equal(r.status,0,r.stderr);const md=readFileSync(out,'utf8');assert.match(md,/0-2\.5秒/);assert.match(md,/2\.5-6秒/);assert.match(md,/系统音/);assert.match(md,/面罩/);assert.equal((md.match(/先别摘下面罩。/g)||[]).length,1);assert.equal(cli(['verify','--input',src,'--output',out]).status,0);assert.notEqual(cli(['export','--input',src,'--output',out]).status,0);}));
bad('拒绝外层非整数片长',b=>b.episodes[0].draft.units[0].shots[0].duration=2.6,'UNIT_INTEGER');
bad('拒绝未兑现选中幻想',b=>{b.episodes[0].plan.fantasyCandidates=[{id:'F1'}];b.episodes[0].selection.selectedIds=['F1'];},'FANTASY_MISSING');
bad('拒绝场级额外引用',b=>b.episodes[0].scenes[1].bindings.push('铜钥匙PR'),'SCENE_BINDINGS');
bad('拒绝缺失集场',b=>b.episodes[0].scenes.pop(),'SCENE_COVERAGE');
bad('拒绝原句倒序',b=>b.episodes[0].draft.units.reverse(),'SCENE_ORDER');
bad('拒绝重复镜号',b=>b.episodes[0].draft.units[0].shots[1].id='S1','SHOT_ID');
bad('拒绝偷改声道',b=>b.episodes[0].draft.units[1].shots[0].dialogueParts[0].presentation='画内','CHANNEL');
bad('拒绝未解决问题',b=>b.unresolved.push({coordinate:'U1',problem:'待决定'}),'UNRESOLVED');
bad('拒绝主观主体冒充现实出场',b=>{b.assets.push({key:'幻影CH',name:'幻影',type:'CH',status:'主观'});b.episodes[0].draft.units[0].visualSubjects=[{name:'幻影',domain:'subjective'}];b.episodes[0].draft.units[0].participants.push('幻影');},'VISUAL_PRESENCE');
for(const [name,replace] of [['正文改字',s=>s.replace('钥匙微动即停','钥匙扔到门外')],['时间篡改',s=>s.replace('0-2.5秒','0-2.6秒')],['引用错绑',s=>s.replace('@[门厅SC]','@[走廊SC]')],['片段缺漏',s=>s.slice(0,s.indexOf('1-2\n'))]])test(`反解发现${name}`,()=>sandbox(dir=>{const src=input(dir),out=join(dir,'final.md');let r=cli(['export','--input',src,'--output',out]);assert.equal(r.status,0,r.stderr);writeFileSync(out,replace(readFileSync(out,'utf8')));r=cli(['verify','--input',src,'--output',out]);assert.notEqual(r.status,0);assert.match(r.stderr,/VERIFY/);}));

bad('拒绝人声错绑资产状态',b=>b.episodes[0].draft.units[0].voiceBindings[0].assetKey='走廊SC','VOICE_BINDING');
bad('拒绝显式对白目录漏掉原声',b=>{b.episodes[0].source.dialogue=[];for(const u of b.episodes[0].draft.units)for(const s of u.shots)s.dialogueParts=[];},'SOURCE_DIALOGUE_COVERAGE');
test('集号以原scope顺序为准，允许特集0在第1集之后',()=>sandbox(dir=>{const b=structuredClone(fixture),e=structuredClone(b.episodes[0]);e.number=0;b.episodes.push(e);b.scope.push({number:0,sceneIds:['1-1','1-2']});const r=cli(['check','--input',input(dir,b)]);assert.equal(r.status,0,r.stderr);}));
test('最终正文不输出内部演出目标、幻想审计与视觉出场审核话语',()=>sandbox(dir=>{const out=join(dir,'final.md'),r=cli(['export','--input',input(dir),'--output',out]);assert.equal(r.status,0,r.stderr);assert.doesNotMatch(readFileSync(out,'utf8'),/演出目标：|幻想边界：|主观视觉主体：|仅视觉呈现，不构成现实出场/);}));
test('内部intent修改不会被回读当成可见内容变化',()=>sandbox(dir=>{const src=input(dir),out=join(dir,'final.md');assert.equal(cli(['export','--input',src,'--output',out]).status,0);const b=structuredClone(fixture);b.episodes[0].draft.units[0].intent.landing='内部审核定位更新';const r=cli(['verify','--input',input(dir,b),'--output',out]);assert.equal(r.status,0,r.stderr);}));
bad('拒绝现场对白冒充系统音',b=>b.episodes[0].draft.units[0].shots[0].dialogueParts[0].presentation='系统音','CHANNEL');

test('四类源声道与七类呈现的兼容矩阵完整覆盖',()=>{
 const allowed={'现场对白':['画内','画外','画外转画内','画内转画外'],'VO':['VO'],'OS':['OS'],'系统音':['系统音','画外']};
 for(const [channel,valid] of Object.entries(allowed))for(const presentation of ['画内','画外','画外转画内','画内转画外','VO','OS','系统音']){
  const b=structuredClone(fixture),e=b.episodes[0],u=e.draft.units[0];
  e.source.sourceRows[1].text=`值守者${channel==='现场对白'?'':`（${channel}）`}：门还没开，先别摘下面罩。`;
  if(channel==='系统音')u.voiceBindings=[];
  for(const shot of u.shots)for(const p of shot.dialogueParts)p.presentation=presentation;
  if(valid.includes(presentation))assert.doesNotThrow(()=>checkBundle(b),`${channel}→${presentation}`);
  else assert.throws(()=>checkBundle(b),error=>error.code==='CHANNEL',`${channel}→${presentation}`);
 }
});

test('经符号链接与系统临时绝对路径启动均真实执行CLI、产文件且拒绝篡改',()=>{
 const dir=mkdtempSync(join(tmpdir(),'ssd-entry-'));
 try{
  const actual=join(dir,'actual'),alias=join(dir,'linked');cpSync(join(base,'scripts'),join(actual,'scripts'),{recursive:true});symlinkSync(actual,alias,process.platform==='win32'?'junction':'dir');
  const src=input(dir),out=join(dir,'final.md');
  const run=(entry,args)=>spawnSync(process.execPath,[join(entry,'scripts/storyboard.mjs'),...args],{encoding:'utf8'});
  for(const entry of [alias,actual]){const r=run(entry,['check','--input',src]);assert.equal(r.status,0,r.stderr);assert.match(r.stdout,/\S/,'必须有JSON输出，不能静默exit0');assert.equal(JSON.parse(r.stdout).ok,true);}
  let r=run(alias,['export','--input',src,'--output',out]);assert.equal(r.status,0,r.stderr);assert.equal(JSON.parse(r.stdout).ok,true);assert.equal(existsSync(out),true);
  r=run(actual,['verify','--input',src,'--output',out]);assert.equal(r.status,0,r.stderr);assert.equal(JSON.parse(r.stdout).ok,true);
  writeFileSync(out,readFileSync(out,'utf8').replace('钥匙微动即停','钥匙扔到门外'));r=run(alias,['verify','--input',src,'--output',out]);assert.notEqual(r.status,0);assert.match(r.stderr,/VERIFY/);
 }finally{rmSync(dir,{recursive:true,force:true});}
});
test('inspect-source独立返回规范化ID与原声字段，无需制作bundle',()=>sandbox(dir=>{const src=join(dir,'source.json');writeFileSync(src,JSON.stringify(fixture.episodes[0].source));const r=cli(['inspect-source','--input',src]);assert.equal(r.status,0,r.stderr);const normalized=JSON.parse(r.stdout);assert.deepEqual(normalized.dialogue.map(q=>q.id),['E01:R2:D1','E01:R4:D2']);assert.equal(normalized.dialogue[0].speaker,'值守者');assert.equal(normalized.dialogue[0].kind,'speech');assert.equal(typeof normalized.dialogue[0].cue,'string');assert.ok(Array.isArray(normalized.sourceRows));assert.ok(Array.isArray(normalized.diagnostics));}));
for(const [cue,quote,parts,category] of [['压低声音','门还没开，先别摘下面罩。',['门还没开，','先别摘下面罩。'],'口语'],['画外咆哮','吼！',['吼','！'],'兽吼'],['尖叫','啊！',['啊','！'],'尖叫'],['低哼','哼！',['哼','！'],'非词人声']])test(`保留源原声类别及提示：${category}`,()=>sandbox(dir=>{const b=structuredClone(fixture),e=b.episodes[0];e.source.sourceRows[1].text=`值守者（${cue}）：${quote}`;for(const [i,s] of e.draft.units[0].shots.entries()){s.dialogueParts[0].text=parts[i];s.dialogueParts[0].presentation='画外';}const src=input(dir,b),out=join(dir,'final.md');let r=cli(['export','--input',src,'--output',out]);assert.equal(r.status,0,r.stderr);const md=readFileSync(out,'utf8');assert.match(md,new RegExp(`原声类别：值守者：${category}`));assert.match(md,new RegExp(`原声提示：值守者：${cue}`));assert.doesNotMatch(md,/原声类别：.*(?:speech|roar|scream|vocalization)/);writeFileSync(out,md.replace(`原声提示：值守者：${cue}`,'原声提示：值守者：被改写提示'));r=cli(['verify','--input',src,'--output',out]);assert.notEqual(r.status,0);assert.match(r.stderr,/VERIFY/);}));

const roundTrip=b=>{const md=renderProjection(projectBundle(b));assert.equal(verifyMarkdown(b,md).ok,true);return md;};
function moveVoiceToPreviousScene(b){
 const [first,last]=b.episodes[0].draft.units,shot=structuredClone(last.shots[0]);
 shot.id='S4';shot.sourceRefs=['E01:R1'];shot.picture='值守者仍在门厅的门左侧等待。';
 first.shots.push(shot);last.shots[0].dialogueParts=[];
}
function oneOriginalScene(b){
 const e=b.episodes[0];b.scope[0].sceneIds=['1-1'];
 e.scenes=[{...e.scenes[0],sourceRefs:e.scenes.flatMap(s=>s.sourceRefs),bindings:[...new Set(e.scenes.flatMap(s=>s.bindings))]}];
 for(const u of e.draft.units)u.sceneId='1-1';
}
function eventPlan(b){
 const e=b.episodes[0],emotionContract={subject:'值守者',contextTone:'等待',audienceEffect:'紧张',change:'确认',landing:'继续等待',retain:['源信息']};
 e.plan.unitPackingPolicy='event-first-v1';
 e.plan.events=[['E01:R1','E01:R2'],['E01:R3','E01:R4']].map((sourceRefs,i)=>({id:`EV${i+1}`,sourceRefs,sceneKey:['门厅','走廊'][i],emotionContract,boundaryBefore:{kind:i?'time_space_jump':'episode_start',reason:'源场景转换'}}));
 for(const [i,u] of e.draft.units.entries()){u.eventId=`EV${i+1}`;u.boundaryBefore=structuredClone(e.plan.events[i].boundaryBefore);}
}
function selectedFantasy(b){
 const e=b.episodes[0];e.plan.fantasyCandidates=[{id:'F1',recommended:true}];e.selection.selectedIds=['F1'];
 e.draft.fantasyRealization=[{candidateId:'F1',status:'realized',unitIds:['U1'],shotIds:['S1'],screenSeconds:1,entry:'主观形态进入',exit:'接回同次动作',domain:'subjective',preservedFunction:'放大提醒'}];
}
bad('显式目录不能改写源原声提示',b=>{const e=b.episodes[0];e.source.sourceRows[1].text='值守者（压低声音）：门还没开，先别摘下面罩。';e.source.dialogue=normalizeSource(e.source).dialogue;e.source.dialogue[0].cue='放声大笑';},'SOURCE_DIALOGUE_BINDING');
bad('显式目录不能把原兽吼降为口语',b=>{const e=b.episodes[0];e.source.sourceRows[1].text='值守者（画外咆哮）：吼！';e.source.dialogue=normalizeSource(e.source).dialogue;e.source.dialogue[0].kind='speech';e.source.dialogue[0].cue='';for(const [i,s] of e.draft.units[0].shots.entries())s.dialogueParts[0].text=['吼','！'][i];},'SOURCE_DIALOGUE_BINDING');
bad('拒绝挂源引用却将原句挪至另一原场发声',moveVoiceToPreviousScene,'DIALOGUE_SOURCE_SCENE');
bad('拒绝含创作内容却没有镜号的分段',b=>{const u=b.episodes[0].draft.units[0];u.segments=[{title:u.title,lighting:u.lighting,initialState:u.initialState,exitState:u.exitState,shotIds:['S1','S2']},{title:'不能静默丢弃',lighting:'红光骤灭',initialState:'手伸向门锁',exitState:'手已收回',shotIds:[]}];},'SEGMENTS');
test('原场内换地保留原场号、场级多SC并集和每单元单SC',()=>{
 const b=structuredClone(fixture);oneOriginalScene(b);const md=roundTrip(b);
 assert.equal((md.match(/^1-1$/gm)||[]).length,1);assert.doesNotMatch(md,/^1-2$/m);assert.match(md,/^场景：门厅、走廊$/m);
 assert.match(md,/门厅是@\[门厅SC\]/);assert.match(md,/走廊是@\[走廊SC\]/);
});
test('同原场跨单元连续原声保留逐段文字和声窗',()=>{
 const b=structuredClone(fixture),e=b.episodes[0],first=e.draft.units[0],next=structuredClone(first);
 next.id='U1B';next.shots=[next.shots[1]];next.shots[0].duration=4;
 first.shots=[first.shots[0]];first.shots[0].duration=4;e.draft.units.splice(1,0,next);
 const md=roundTrip(b);assert.equal((md.match(/先别摘下面罩。/g)||[]).length,1);assert.match(md,/本条0-3\.2秒连续原声/);
});
for(const id of ['[1-1]','第2集','片段2','@场、无','内容：原场','0-1秒：','  原场'])test(`原场标签逐字可回读：${id}`,()=>{
 const b=structuredClone(fixture);b.scope[0].sceneIds[1]=id;b.episodes[0].scenes[1].id=id;b.episodes[0].draft.units[1].sceneId=id;
 assert.ok(roundTrip(b).includes(`\n${id}\n场景：`));
});
bad('拒绝未知幻想模式',b=>b.episodes[0].settings.fantasyMode='nonsense','FANTASY_MODE');
bad('显式null幻想模式不当作未给默认值',b=>b.episodes[0].settings.fantasyMode=null,'FANTASY_MODE');
bad('旧fantasyEnabled不能与正式幻想模式冲突',b=>b.episodes[0].settings.fantasyEnabled=false,'FANTASY_MODE');
bad('旧fantasyEnabled若保留必须为布尔值',b=>b.episodes[0].settings.fantasyEnabled='true','FANTASY_MODE');
bad('off拒绝额外幻想已选与兑现记录',b=>{selectedFantasy(b);b.episodes[0].settings.fantasyMode='off';},'FANTASY_MODE');
bad('all不能漏选实际候选',b=>{const e=b.episodes[0];e.settings.fantasyMode='all';e.plan.fantasyCandidates=[{id:'F1',recommended:false}];},'FANTASY_MODE');
bad('拒绝重复幻想候选ID',b=>b.episodes[0].plan.fantasyCandidates=[{id:'F1'},{id:'F1'}],'FANTASY_CANDIDATE');
test('默认recommended保留显式选择且不自动填选',()=>{
 const b=structuredClone(fixture);b.episodes[0].plan.fantasyCandidates=[{id:'F1',recommended:true}];const original=structuredClone(b);
 roundTrip(b);assert.deepEqual(b,original);assert.deepEqual(b.episodes[0].selection.selectedIds,[]);
});
test('review接受显式空选择，all接受实际候选全选',()=>{
 const reviewed=structuredClone(fixture);reviewed.episodes[0].settings.fantasyMode='review';roundTrip(reviewed);
 const all=structuredClone(fixture);selectedFantasy(all);all.episodes[0].settings.fantasyMode='all';roundTrip(all);
});
test('off仍可保留原文明确Q版而不新增幻想记录',()=>{
 const b=structuredClone(fixture),e=b.episodes[0];e.settings.fantasyMode='off';e.settings.fantasyEnabled=false;e.source.sourceRows[0].text+='原文明示本段为Q版。';e.draft.units[0].shots[0].picture+='保留源要求的Q版。';
 assert.match(roundTrip(b),/保留源要求的Q版/);
});
bad('拒绝未知事件策略静默跳过归属校验',b=>{const e=b.episodes[0];e.plan.unitPackingPolicy='event-first-v2';e.plan.events=[];for(const u of e.draft.units)u.eventId='不存在';},'PLAN_EVENT_POLICY');
bad('拒绝不同场景事件声明连续边界',b=>{eventPlan(b);b.episodes[0].plan.events[1].boundaryBefore.kind='continuous';b.episodes[0].draft.units[1].boundaryBefore.kind='continuous';},'PLAN_EVENT_BOUNDARY');
bad('拒绝事件源范围重复归属',b=>{eventPlan(b);b.episodes[0].plan.events[1].sourceRefs.unshift('E01:R2');},'PLAN_EVENT');
bad('拒绝同原场原声跨独立事件挪播',b=>{eventPlan(b);oneOriginalScene(b);moveVoiceToPreviousScene(b);},'DIALOGUE_EVENT');
test('事件源覆盖与明确时空边界可回读',()=>{const b=structuredClone(fixture);eventPlan(b);roundTrip(b);});

test('CRLF冻结源仍完整核事件覆盖，漏行仍拒绝且不改原文',()=>{
 const b=structuredClone(fixture);eventPlan(b);
 const e=b.episodes[0],oldSource=e.source;
 const source=prepareSource(oldSource.sourceRows.map(r=>r.text).join('\r\n')+'\r\n','E01',['值守者','系统音']);
 const refs=new Map(oldSource.sourceRows.map((r,i)=>[r.id,source.sourceRows[i].id]));
 const voices=new Map(normalizeSource(oldSource).dialogue.map((q,i)=>[q.id,source.dialogue[i].id]));
 for(const scene of e.scenes)scene.sourceRefs=scene.sourceRefs.map(r=>refs.get(r));
 for(const event of e.plan.events)event.sourceRefs=event.sourceRefs.map(r=>refs.get(r));
 for(const unit of e.draft.units){
  for(const v of unit.voiceBindings)v.lineId=voices.get(v.lineId);
  for(const shot of unit.shots){shot.sourceRefs=shot.sourceRefs.map(r=>refs.get(r));for(const part of shot.dialogueParts)part.lineId=voices.get(part.lineId);}
 }
 e.source=source;assert.ok(source.sourceRows.every(r=>r.text.endsWith('\r\n')));
 const lock={version:'series-source-lock-v1',sourceHash:'a'.repeat(64),episodes:b.episodes.map(({number,source})=>({number,source:structuredClone(source)}))};
 b.sourceLockHash=createHash('sha256').update(JSON.stringify(lock)).digest('hex');b.generationTextVersion='voice-delivery-v1';
 const before=JSON.stringify(b);
 assert.equal(checkBundle(b,{sourceLock:lock}).ok,true);
 const md=renderProjection(projectBundle(b,{sourceLock:lock}));assert.equal(verifyMarkdown(b,md,{sourceLock:lock}).ok,true);
 assert.equal(JSON.stringify(b),before);
 b.episodes[0].plan.events[1].sourceRefs=[refs.get('E01:R4')];
 assert.throws(()=>checkBundle(b,{sourceLock:lock}),error=>error.code==='PLAN_EVENT_COVERAGE');
});
test('不同事件可共享源元信息而不重复实际剧情归属',()=>{
 const b=structuredClone(fixture);eventPlan(b);oneOriginalScene(b);const e=b.episodes[0];
 e.source.sourceRows.unshift({id:'E01:R0',text:'场景：门厅与走廊'});e.scenes[0].sourceRefs.unshift('E01:R0');
 for(const event of e.plan.events)event.sourceRefs.unshift('E01:R0');roundTrip(b);
});
bad('超9位有效镜长精度不能check过后导出失败',b=>b.episodes[0].draft.units[0].shots[0].duration=2.5000000006,'TIME_PRECISION');
bad('不能将非零极小声窗静默取整为0',b=>{const p=b.episodes[0].draft.units[0].shots[0].dialogueParts[0];p.voiceStart=1e-10;p.voiceEnd=2e-10;},'TIME_PRECISION');
bad('纳秒精度声窗也不能越出本镜',b=>b.episodes[0].draft.units[0].shots[0].dialogueParts[0].voiceEnd=2.500000001,'VOICE_WINDOW');
test('9位小数声窗可回读且机器浮点噪声不误报',()=>{
 const b=structuredClone(fixture),shots=b.episodes[0].draft.units[0].shots;shots[0].dialogueParts[0].voiceStart=1e-9;shots[0].dialogueParts[0].voiceEnd=2e-9;
 assert.match(roundTrip(b),/本条0\.000000001-0\.000000002秒连续原声/);
 const noise=structuredClone(fixture);noise.episodes[0].draft.units[0].shots[0].dialogueParts[0].voiceStart=0.1+0.2;
 assert.match(roundTrip(noise),/本条0\.3-2\.3秒连续原声/);
});
test('同一人物可同时作为现场人物和有源屏幕主体保留CH绑定',()=>{
 const b=structuredClone(fixture),e=b.episodes[0],u=e.draft.units[0];
 e.source.sourceRows[0].text+='门边监控屏显示值守者本人。';u.mediaSubjects=[{name:'值守者·面罩',medium:'screen',sourceRefs:['E01:R1']}];
 u.shots[0].picture+='门边监控屏内显示同一值守者，与屏外本人明确分层。';
 const original=structuredClone(b),md=roundTrip(b);assert.deepEqual(b,original);assert.deepEqual(u.participants,['值守者·面罩']);
 assert.equal((md.match(/^人物：值守者·面罩$/gm)||[]).length,1);assert.match(md,/监控屏内显示同一值守者/);assert.doesNotMatch(md,/mediaSubjects|screen|媒体主体：/);
});
test('仅照片主体可绑定CH且不成为现实出场者',()=>{
 const b=structuredClone(fixture),e=b.episodes[0],u=e.draft.units[1];
 e.source.sourceRows[2].text+='墙上照片是戴面罩的值守者。';u.mediaSubjects=[{name:'值守者·面罩',medium:'photo',sourceRefs:['E01:R3']}];
 u.bindings.push('值守者·面罩CH');e.scenes[1].bindings.push('值守者·面罩CH');u.shots[0].picture+='墙上照片显示戴面罩的值守者，走廊现场无人。';
 assert.match(roundTrip(b),/墙上照片显示戴面罩的值守者/);assert.deepEqual(u.participants,[]);assert.deepEqual(u.visualSubjects,[]);
});
bad('媒体主体不能用其他原场来源自证',b=>b.episodes[0].draft.units[0].mediaSubjects=[{name:'值守者·面罩',medium:'screen',sourceRefs:['E01:R3']}],'MEDIA_SUBJECT');
bad('媒体主体不能引用不存在的来源',b=>b.episodes[0].draft.units[0].mediaSubjects=[{name:'值守者·面罩',medium:'screen',sourceRefs:['E01:UNKNOWN']}],'MEDIA_SUBJECT');
bad('拒绝未绑定的媒体人物与未知媒体域',b=>b.episodes[0].draft.units[0].mediaSubjects=[{name:'未绑定人物',medium:'hologram',sourceRefs:['E01:R1']}],'MEDIA_SUBJECT');
test('同一原场内单元日夜内外覆盖逐字导出且篡改可发现',()=>{
 const b=structuredClone(fixture);oneOriginalScene(b);const e=b.episodes[0],u=e.draft.units[1];
 e.source.sourceRows[2].text='△回忆中，白天的露天长廊空无一人。';u.dayInterior='日 外';u.initialState='回忆中的白天露天长廊无人。';u.shots[0].picture='回忆中的露天长廊在日光下空无一人。';
 const projected=projectBundle(b),md=roundTrip(b);assert.deepEqual(projected[0].scenes[0].units.map(u=>u.dayInterior),['夜 内','日 外']);
 assert.match(md,/片段2\n时长：4秒\n日 外/);assert.throws(()=>verifyMarkdown(b,md.replace('\n日 外\n','\n夜 内\n')),error=>error.code==='VERIFY_MISMATCH');
});
bad('拒绝无效单元日夜内外而不忽略字段',b=>b.episodes[0].draft.units[1].dayInterior='白天 室外','DAY_INTERIOR');
