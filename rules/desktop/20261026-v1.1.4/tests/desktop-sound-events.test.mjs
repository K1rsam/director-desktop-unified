import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdtemp} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {tmpdir} from 'node:os';
import {join,resolve,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {normalizeSource} from '../scripts/core/runtime/storyboard-v4/core/source.mjs';
import {validateDraft,renderDraft} from '../scripts/core/runtime/storyboard-v4/core/quality.mjs';
import {buildOutputSchema} from '../scripts/core/runtime/storyboard-v4/core/output-schema.mjs';
import {baseDraftHash,applyDelta} from '../scripts/core/runtime/storyboard-v4/core/delta.mjs';
import {SOUND_EVENT_PROFILE,soundEventSourceBinding,validateSoundEvents} from '../scripts/core/runtime/storyboard-v4/core/sound-events.mjs';
import {inspectDialogue,materializeDialogue,verifyMaterialized} from '../scripts/materialize-dialogue.mjs';
import {freezeSource,checkBundle,projectBundle,renderProjection,verifyMarkdown} from '../scripts/storyboard.mjs';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const fail=(fn,code)=>assert.throws(fn,error=>error.code===code);
const rawSource={key:'E1',sourceRows:[
 {id:'E1:R0',text:'△门厅里，甲停步看向门板。'},
 {id:'E1:R1',text:'环境（VO）：门外传来三下撞击声。'},
 {id:'E1:R2',text:'甲：有人来了。'},
 {id:'E1:R3',text:'甲（OS）：先别开门。'},
 {id:'E1:R4',text:'系统音：门锁正常。'},
 {id:'E1:R5',text:'旁白（VO）：他想起了故乡。'}
],contextRows:[]};
function fixture(){
 const source=structuredClone(rawSource),normalized=normalizeSource(source),qs=normalized.dialogue;
 const parts=qs.map((q,i)=>({lineId:q.id,text:q.quote,presentation:i===0?'声音事件':q.channel==='现场对白'?'画内':q.channel,
  delivery:i===0?'门板连续响起三记沉闷撞击，第二记较重，余响迅速衰减。':'保持原句自然语气。',
  voiceStart:[0.2,0.4,0,1.1,2.1][i],voiceEnd:[2.2,3,1,2,3.8][i],
  ...(i===0?{sourceQuoteRange:[0,q.quote.length]}:{})}));
 const draft={schemaVersion:'performance-core-v4',fantasyRealization:[],unresolved:[],
  soundEventDecisions:{version:SOUND_EVENT_PROFILE,items:[{lineId:qs[0].id,...soundEventSourceBinding(normalized,qs[0]),
   interpretation:'non_speech_sound',reason:'本句说明门外撞击的实际声音；没有说话、叙述、内心话或系统播报，按门板受击的声响演绎。'}]},
  units:[{id:'U1',title:'门外的声音',sceneId:'1-1',scene:'门厅',props:[],participants:['甲'],
   lighting:'门上方暖灯稳定。',initialState:'甲站在门内半步，双手垂下。',exitState:'甲保持门内位置，门锁完好。',
   intent:{subject:'甲',change:'听见门外撞击后警觉',landing:'停在门内确认门锁',retain:['门外声响和所有原句']},
   bindings:['门厅SC','甲CH','旁白CH'],imageBindings:['门厅SC','甲CH'],
   voiceBindings:[{lineId:qs[1].id,assetKey:'甲CH'},{lineId:qs[2].id,assetKey:'甲CH'},{lineId:qs[4].id,assetKey:'旁白CH'}],
   shots:[{id:'S1',duration:4,camera:'中景固定，甲与门板同框。',picture:'甲停步，侧耳看向门板。',lighting:'暖灯照清侧脸。',sound:'室内底噪持续。',transition:'接甲的凝视。',sourceRefs:['E1:R0','E1:R1','E1:R2'],dialogueParts:parts.slice(0,2)},
    {id:'S2',duration:4,camera:'近景看甲的眼睛。',picture:'甲视线落向门锁，身体仍停在原位。',lighting:'门灯维持同向暖光。',sound:'门外余响消退。',transition:'视线落稳后切出。',sourceRefs:['E1:R3','E1:R4','E1:R5'],dialogueParts:parts.slice(2)}]}]};
 return {version:'series-storyboard-bundle-v1',exportProfile:'lingmo-v7',testFixture:true,title:'合成声音事件测试',unresolved:[],
  scope:[{number:1,sceneIds:['1-1']}],assets:[{name:'门厅',key:'门厅SC',type:'SC',status:'常态',importName:'门厅',placeholderOrdinal:0},
   {name:'甲',key:'甲CH',type:'CH',status:'常态',importName:'甲',placeholderOrdinal:1},{name:'旁白',key:'旁白CH',type:'CH',status:'纯声源',importName:'旁白',placeholderOrdinal:2}],
  episodes:[{number:1,source,plan:{fantasyCandidates:[]},selection:{selectedIds:[]},settings:{visualStyle:'写实、清晰的门厅。',prohibitions:'无额外字幕、配乐、水印。',maxUnitSeconds:30},
   scenes:[{id:'1-1',dayInterior:'夜 内',sourceRefs:source.sourceRows.map(row=>row.id),bindings:['门厅SC','甲CH','旁白CH']}],draft}]};
}
function selection(bundle){
 const ep=bundle.episodes[0],bytes=Buffer.from(JSON.stringify(ep.source)),info=inspectDialogue(bytes),draft=structuredClone(ep.draft),cursor=new Map();
 for(const unit of draft.units)for(const shot of unit.shots)for(const part of shot.dialogueParts){
  const start=cursor.get(part.lineId)??0,end=start+part.text.length;cursor.set(part.lineId,end);
  delete part.text;delete part.sourceQuoteRange;part.quoteStart=start;part.quoteEnd=end;
 }
 return {bytes,input:{version:'desktop-dialogue-selection-v1',coordinateProfile:info.coordinateProfile,sourceSha256:info.sourceSha256,draft}};
}
const render=b=>renderProjection(projectBundle(b));

test('desktop materialized sound events coexist with independent source lock and new text policy',()=>{
 const b=fixture(),lock=freezeSource({sourceHash:'a'.repeat(64),episodes:b.episodes});
 b.sourceLockHash=createHash('sha256').update(JSON.stringify(lock)).digest('hex');
 b.generationTextVersion='voice-delivery-v1';
 const {bytes,input}=selection(b);b.episodes[0].draft=materializeDialogue(bytes,input).draft;
 const options={sourceLock:lock},md=renderProjection(projectBundle(b,options));
 assert.equal(checkBundle(b,options).sourceFidelity.mode,'locked-source-v1');
 assert.equal(verifyMarkdown(b,md,options).ok,true);
 assert.match(md,/非人声：门板连续响起三记沉闷撞击/);
 for(const quote of ['有人来了。','先别开门。','门锁正常。','他想起了故乡。'])assert.ok(md.includes(quote));
 assert.ok(!md.includes('门外传来三下撞击声。'));
 const changed=structuredClone(b);
 changed.episodes[0].source.sourceRows[1].text='环境（VO）：门外传来两下撞击声。';
 // Even rematerializing a new, internally consistent event cannot replace intake truth.
 const info=inspectDialogue(Buffer.from(JSON.stringify(changed.episodes[0].source)));
 changed.episodes[0].draft.soundEventDecisions.items[0]={...changed.episodes[0].draft.soundEventDecisions.items[0],
  quoteSha256:info.dialogue[0].quoteSha256,sourceBindingSha256:info.dialogue[0].sourceBindingSha256};
 changed.episodes[0].draft.units[0].shots[0].dialogueParts[0].text=info.dialogue[0].quote;
 assert.throws(()=>checkBundle(changed,options),error=>error.code==='SOURCE_LOCK_SOURCE');
});

test('explicit sound event preserves locked source and coexists with real dialogue, OS, system and narration',()=>{
 const b=fixture(),before=structuredClone(b.episodes[0].source),md=render(b),ep=b.episodes[0];
 assert.deepEqual(ep.source,before);assert.equal(checkBundle(b).ok,true);assert.equal(verifyMarkdown(b,md).ok,true);
 assert.match(md,/声音：室内底噪持续。\n  0.2–2.2秒｜非人声：门板连续响起三记沉闷撞击/);
 assert.ok(!md.includes('门外传来三下撞击声。'),'追溯描述不被朗读或泄入媒体正文');
 for(const quote of ['有人来了。','先别开门。','门锁正常。','他想起了故乡。'])assert.ok(md.includes(quote));
 assert.ok(!md.includes('quoteSha256')&&!md.includes('non_speech_sound')&&!md.includes('本句说明'));
 assert.equal(ep.draft.units[0].voiceBindings.length,3,'事件不虚造环境声CH');
 const result=validateDraft({draft:ep.draft,source:normalizeSource(ep.source),settings:ep.settings,plan:ep.plan,selection:ep.selection});
 assert.deepEqual(result.errors,[]);
 assert.match(renderDraft({draft:ep.draft,source:normalizeSource(ep.source),settings:ep.settings}),/非人声：门板连续响起/);
});
test('event-only shot has 台词：无 and independently verified sound delivery and time',()=>{
 const b=fixture(),u=b.episodes[0].draft.units[0],speech=u.shots[0].dialogueParts.pop();
 u.shots.splice(1,0,{...structuredClone(u.shots[0]),id:'S1b',dialogueParts:[speech],sourceRefs:['E1:R2']});
 const md=render(b);assert.match(md,/非人声：[^\n]+\n台词：无/);assert.equal(verifyMarkdown(b,md).ok,true);
 for(const [from,to] of [['非人声：门板','非人声：地面'],['0.2–2.2秒｜非人声','0.2–2.3秒｜非人声']])fail(()=>verifyMarkdown(b,md.replace(from,to)),'VERIFY_MISMATCH');
});
test('materialize and verify retain the explicit decision and exact source intervals',()=>{
 const b=fixture(),{bytes,input}=selection(b),result=materializeDialogue(bytes,input),q=inspectDialogue(bytes).dialogue[0];
 assert.equal(verifyMaterialized(bytes,result).ok,true);assert.deepEqual(result.draft.soundEventDecisions,input.draft.soundEventDecisions);
 assert.deepEqual(result.draft.units[0].shots[0].dialogueParts[0].sourceQuoteRange,[0,q.utf16Length]);
 assert.equal(result.draft.units[0].shots[0].dialogueParts[0].text,q.quote);
 b.episodes[0].draft=result.draft;assert.equal(verifyMarkdown(b,render(b)).ok,true);
 assert.equal(q.quoteSha256.length,64);assert.equal(q.sourceBindingSha256.length,64);assert.deepEqual(q.sourceRefs,['E1:R1']);
 const bad=structuredClone(result);bad.draft.soundEventDecisions.items[0].reason='替换判断';fail(()=>verifyMaterialized(bytes,bad),'DRAFT_DRIFT');
});
test('event may span consecutive shots while preserving one continuous source quote',()=>{
 const b=fixture(),u=b.episodes[0].draft.units[0],first=u.shots[0],event=first.dialogueParts[0],cut=4;
 const next={...structuredClone(first),id:'S1b'};
 first.dialogueParts=[{...event,text:event.text.slice(0,cut),sourceQuoteRange:[0,cut]}];
 next.dialogueParts[0]={...event,text:event.text.slice(cut),sourceQuoteRange:[cut,event.text.length],delivery:'门板余下两次受击，最后一记余响迅速衰减。'};
 u.shots.splice(1,0,next);
 const {bytes,input}=selection(b),result=materializeDialogue(bytes,input);
 assert.equal(verifyMaterialized(bytes,result).ok,true);b.episodes[0].draft=result.draft;
 const md=render(b);assert.equal(verifyMarkdown(b,md).ok,true);
 assert.match(md,/4.2–6.2秒｜非人声：门板余下两次受击/);
 const parts=result.draft.units[0].shots.slice(0,2).map(s=>s.dialogueParts[0]);
 assert.equal(parts.map(p=>p.text).join(''),event.text);
 assert.deepEqual(parts.map(p=>p.sourceQuoteRange),[[0,cut],[cut,event.text.length]]);
});
test('event ranges cannot split a Unicode pair and direct validators reject unauthorised events',()=>{
 const b=fixture(),ep=b.episodes[0];ep.source.sourceRows[1].text='环境（VO）：砰😀，门板震响。';
 const source=normalizeSource(ep.source),q=source.dialogue[0],part=ep.draft.units[0].shots[0].dialogueParts[0];
 ep.draft.soundEventDecisions.items[0]={...ep.draft.soundEventDecisions.items[0],...soundEventSourceBinding(source,q)};
 part.text=q.quote;part.sourceQuoteRange=[0,q.quote.length];assert.equal(checkBundle(b).ok,true);
 const bad=structuredClone(ep.draft),cut=2;bad.units[0].shots[0].dialogueParts[0].sourceQuoteRange=[0,cut];
 bad.units[0].shots[0].dialogueParts[0].text=q.quote.slice(0,cut);
 fail(()=>validateSoundEvents(bad,source),'SOUND_EVENT_RANGE');
 const {bytes,input}=selection(b);input.draft.units[0].shots[0].dialogueParts[0].quoteEnd=cut;
 fail(()=>materializeDialogue(bytes,input),'UTF16_BOUNDARY');
 delete ep.draft.soundEventDecisions;
 const result=validateDraft({draft:ep.draft,source,settings:ep.settings,plan:ep.plan,selection:ep.selection});
 assert.ok(result.errors.some(e=>e.code==='SOUND_EVENT_AUTHORIZATION'));
 fail(()=>renderDraft({draft:ep.draft,source,settings:ep.settings}),'SOUND_EVENT_AUTHORIZATION');
});
test('no implicit authorization: missing, duplicate, wrong profile, classification and evidence are rejected',()=>{
 const cases=[['SOUND_EVENT_AUTHORIZATION',b=>{delete b.episodes[0].draft.soundEventDecisions;}],
  ['SOUND_EVENT_PROFILE',b=>{b.episodes[0].draft.soundEventDecisions.version='guess';}],
  ['SOUND_EVENT_DECISION',b=>{const p=b.episodes[0].draft.soundEventDecisions;p.items.push(structuredClone(p.items[0]));}],
  ['SOUND_EVENT_DECISION',b=>{b.episodes[0].draft.soundEventDecisions.items[0].interpretation='narration';}],
  ['SOUND_EVENT_DECISION',b=>{b.episodes[0].draft.soundEventDecisions.items[0].reason='';}],
  ['SOUND_EVENT_SOURCE_DRIFT',b=>{b.episodes[0].draft.soundEventDecisions.items[0].sourceRefs=['E1:R2'];}],
  ['SOUND_EVENT_SOURCE_DRIFT',b=>{b.episodes[0].draft.soundEventDecisions.items[0].quoteSha256='0'.repeat(64);}],
  ['SOUND_EVENT_INTERPRETATION',b=>{b.episodes[0].draft.units[0].shots[0].dialogueParts[0].presentation='VO';}]];
 for(const [code,mutate]of cases){const b=fixture();mutate(b);fail(()=>checkBundle(b),code);}
 const {bytes,input}=selection(fixture());delete input.draft.soundEventDecisions;fail(()=>materializeDialogue(bytes,input),'SOUND_EVENT_AUTHORIZATION');
});
test('ordinary speech, OS, system voice and existing nonlexical vocal types cannot become sound events',()=>{
 for(const line of ['环境：门外传来三下撞击声。','环境（OS）：门外传来三下撞击声。','系统音：门外传来三下撞击声。','环境（VO咆哮）：吼！','环境（VO尖叫）：啊！','环境（VO低哼）：哼！']){
  const b=fixture(),ep=b.episodes[0];ep.source.sourceRows[1].text=line;
  const source=normalizeSource(ep.source),q=source.dialogue[0];
  ep.draft.soundEventDecisions.items[0]={...ep.draft.soundEventDecisions.items[0],lineId:q.id,...soundEventSourceBinding(source,q)};
  fail(()=>checkBundle(b),'SOUND_EVENT_PROTECTED_VOICE');
 }
});
test('source text, speaker, channel, ranges and materialized text cannot drift',()=>{
 for(const mutate of [ep=>ep.source.sourceRows[1].text+='新字',ep=>ep.source.sourceRows[1].text=ep.source.sourceRows[1].text.replace('环境','异人')]){
  const b=fixture();mutate(b.episodes[0]);fail(()=>checkBundle(b),'SOUND_EVENT_SOURCE_DRIFT');
 }
 const b=fixture();b.episodes[0].draft.units[0].shots[0].dialogueParts[0].text='改字';fail(()=>checkBundle(b),'SOUND_EVENT_TEXT');
 const c=fixture();c.episodes[0].draft.units[0].shots[0].dialogueParts[0].sourceQuoteRange[0]=1;fail(()=>checkBundle(c),'SOUND_EVENT_RANGE');
 const d=fixture();d.episodes[0].draft.units[0].shots[0].dialogueParts[0].speaker='环境';fail(()=>checkBundle(d),'SOUND_EVENT_SOURCE_FIELDS');
 const e=fixture();e.episodes[0].draft.units[0].shots[0].dialogueParts.shift();fail(()=>checkBundle(e),'SOUND_EVENT_COVERAGE');
});
test('event windows must be real and in shot; only events may overlap speech',()=>{
 for(const [start,end]of [[-1,2],[1,1],[2,5],[0,NaN]]){
  const b=fixture(),p=b.episodes[0].draft.units[0].shots[0].dialogueParts[0];p.voiceStart=start;p.voiceEnd=end;fail(()=>checkBundle(b),'SOUND_EVENT_WINDOW');
 }
 const b=fixture();b.episodes[0].draft.units[0].shots[1].dialogueParts[1].voiceStart=0.5;fail(()=>checkBundle(b),'VOICE_WINDOW');
 const c=fixture();c.episodes[0].draft.units[0].voiceBindings.push({lineId:normalizeSource(c.episodes[0].source).dialogue[0].id,assetKey:'旁白CH'});fail(()=>checkBundle(c),'VOICE_BINDING');
});
test('schema opts in only with bound decisions; no generic event enum in legacy schema',()=>{
 const b=fixture(),ep=b.episodes[0],args={stage:'author',source:normalizeSource(ep.source),settings:ep.settings,selection:ep.selection,scenarioKeys:[]};
 const old=buildOutputSchema(args);assert.ok(!JSON.stringify(old).includes('声音事件'));assert.equal(old.properties.soundEventDecisions,undefined);
 const current=buildOutputSchema({...args,soundEventDecisions:ep.draft.soundEventDecisions});
 assert.ok(JSON.stringify(current).includes('sourceQuoteRange'));assert.deepEqual(current.properties.soundEventDecisions.enum,[ep.draft.soundEventDecisions]);
 const b2=fixture();b2.exportProfile='series-single-zone-v1';fail(()=>checkBundle(b2),'SOUND_EVENT_EXPORT_PROFILE');
});
test('legacy bundle validates, renders without sound-event text and independently roundtrips',async()=>{
 const b=JSON.parse(await readFile(join(root,'tests/fixtures/lingmo-v7-bundle.json'),'utf8'));
 assert.equal(checkBundle(b).ok,true);const md=render(b);assert.equal(verifyMarkdown(b,md).ok,true);assert.ok(!md.includes('非人声：'));
});
test('core delta preserves event decisions and binds them into the base hash',()=>{
 const b=fixture(),ep=b.episodes[0],base=ep.draft;
 const delta={schemaVersion:'performance-core-v4-delta',baseDraftHash:baseDraftHash(base),retainedUnitIds:['U1'],replacements:[]};
 const result=applyDelta(base,delta);assert.deepEqual(result.soundEventDecisions,base.soundEventDecisions);
 validateSoundEvents(result,normalizeSource(ep.source));
 const changed=structuredClone(base);changed.soundEventDecisions.items[0].reason='另一依据';fail(()=>applyDelta(changed,delta),'DELTA_STALE');
 fail(()=>applyDelta(base,{...delta,soundEventDecisions:base.soundEventDecisions}),'SOUND_EVENT_DELTA');
});
test('offline CLI build/verify and export/independent verify run end to end',async()=>{
 const dir=await mkdtemp(join(tmpdir(),'desktop-sound-events-')),b=fixture(),{bytes,input}=selection(b);
 const source=join(dir,'source.json'),sel=join(dir,'selection.json'),result=join(dir,'result.json'),bundle=join(dir,'bundle.json'),md=join(dir,'story.md');
 await writeFile(source,bytes);await writeFile(sel,JSON.stringify(input));
 const run=(script,args)=>{const r=spawnSync(process.execPath,[join(root,'scripts',script),...args],{encoding:'utf8',cwd:tmpdir()});assert.equal(r.status,0,r.stdout+r.stderr);return r;};
 run('materialize-dialogue.mjs',['build','--source',source,'--input',sel,'--output',result]);
 run('materialize-dialogue.mjs',['verify','--source',source,'--input',result]);
 b.episodes[0].draft=JSON.parse(await readFile(result,'utf8')).draft;await writeFile(bundle,JSON.stringify(b));
 run('storyboard.mjs',['check','--input',bundle]);run('storyboard.mjs',['export','--input',bundle,'--output',md]);
 run('storyboard.mjs',['verify','--input',bundle,'--output',md]);
 assert.match(await readFile(md,'utf8'),/非人声：门板连续响起/);
});
