#!/usr/bin/env node
import {checkVisualBaselines} from './visual-baseline.mjs';
import {checkGenerationBoundary} from './generation-boundary.mjs';
import {inspectFieldBudget} from './lingmo-field-budget.mjs';
import {readFile,writeFile,mkdir,unlink} from 'node:fs/promises';
import {realpathSync} from 'node:fs';
import {dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {isDeepStrictEqual} from 'node:util';
import {createHash} from 'node:crypto';
import {normalizeSource} from './core/runtime/storyboard-v4/core/source.mjs';
import {validateDraft} from './core/runtime/storyboard-v4/core/quality.mjs';
import {validVisualSubjects} from './core/runtime/storyboard-v4/core/visual-subjects.mjs';
import {checkAssetBindings,readRegistry} from './assets.mjs';
import {validateImportMappings} from './lingmo-asset-names.mjs';
import {SECOND_TICKS,timeTicks,timeText} from './time-values.mjs';
import {projectLingmoV7,renderLingmoV7,verifyLingmoProjection,buildLingmoCatalog} from './lingmo-v7.mjs';
import {checkSeasonBundle} from './season-scope.mjs';

const has=v=>typeof v==='string'&&v.trim()&&!v.includes('\0')&&!v.includes('\r');
const list=v=>Array.isArray(v)?v:[];
const sameSet=(a,b)=>a.length===b.length&&new Set(a).size===a.length&&a.every(x=>b.includes(x));
const fail=(code,message)=>{throw Object.assign(new Error(message),{code});};
const need=(ok,code,message)=>{if(!ok)fail(code,message);};
const names=(keys,assets,type)=>keys.map(k=>assets.get(k)).filter(a=>a.type===type).map(a=>a.name);
const voiceKinds=new Map([['speech','口语'],['roar','兽吼'],['scream','尖叫'],['vocalization','非词人声']]);
const voicePresentations=new Map([['现场对白',new Set(['画内','画外','画外转画内','画内转画外'])],['VO',new Set(['VO'])],['OS',new Set(['OS'])],['系统音',new Set(['系统音','画外'])]]);
const cleanName=v=>has(v)&&!/[\n、\[\]@]/.test(v)&&v!=='无';
const sceneLabel=v=>has(v)&&!v.includes('\n');
const validDayInterior=v=>typeof v==='string'&&/^(日|夜|晨|午|黄昏|待确认) (内|外|待确认)$/.test(v);

function visualBindings(unit,subjects,assets){
 const bindings=unit.visualSubjectBindings??[];
 need(Array.isArray(bindings)&&bindings.length===subjects.length,'VISUAL_BINDINGS',`${unit.id} 每个视觉主体须明确选择正式资产或正文表现(null)`);
 const seen=new Set();
 for(const b of bindings){
  need(b&&Object.keys(b).every(k=>['name','domain','assetKey'].includes(k))&&subjects.some(s=>s.name===b.name&&s.domain===b.domain),'VISUAL_BINDINGS',`${unit.id} 视觉主体映射无对应主体或含未知字段`);
  const identity=JSON.stringify([b.name,b.domain]);need(!seen.has(identity),'VISUAL_BINDINGS',`${unit.id} 视觉主体映射重复`);seen.add(identity);
  need(b.assetKey===null||(assets.get(b.assetKey)?.type==='CH'&&unit.bindings.includes(b.assetKey)),'VISUAL_BINDINGS',`${unit.id} 视觉主体须指向本单元CH或明确null`);
 }
 return bindings;
}

function checkedEvents(plan,source){
 if(plan.unitPackingPolicy===undefined)return null;
 need(plan.unitPackingPolicy==='event-first-v1','PLAN_EVENT_POLICY','未知事件单元策略');
 need(Array.isArray(plan.events)&&plan.events.length,'PLAN_EVENTS','完整事件目录缺失');
 // Source rows preserve original CRLF; the single-line output-field predicate
 // must not erase every production row from an otherwise valid source lock.
 const required=source.sourceRows.filter(r=>typeof r.text==='string'&&r.text.trim()&&!['blank','punctuation','metadata'].includes(r.kind)&&!/^\s*[！!？?。，,.、…；;：:]+\s*$/.test(r.text)).map(r=>r.id);
 const rows=new Map(source.sourceRows.map((r,i)=>[r.id,i])),events=new Map(),owned=new Set();let previousEnd=-1;
 for(const [i,event] of plan.events.entries()){
  need(event&&has(event.id)&&!events.has(event.id)&&has(event.sceneKey),'PLAN_EVENT','事件标识或场景缺失、重复');
  need(['episode_start','continuous','new_event','time_space_jump'].includes(event.boundaryBefore?.kind)&&has(event.boundaryBefore?.reason)&&(i===0)===(event.boundaryBefore.kind==='episode_start'),'PLAN_EVENT_BOUNDARY',`${event.id} 事件边界或首事件归属无效`);
  need(!(i&&event.boundaryBefore.kind==='continuous'&&event.sceneKey!==plan.events[i-1].sceneKey),'PLAN_EVENT_BOUNDARY',`${event.id} 不同场景事件不能声明continuous`);
  need(Array.isArray(event.sourceRefs)&&event.sourceRefs.length&&new Set(event.sourceRefs).size===event.sourceRefs.length&&event.sourceRefs.every((ref,j)=>rows.has(ref)&&(!j||rows.get(ref)>rows.get(event.sourceRefs[j-1]))),'PLAN_EVENT_REFS',`${event.id} 事件源引用未知、重复或倒序`);
  const indices=event.sourceRefs.filter(ref=>required.includes(ref)).map(ref=>required.indexOf(ref));
  need(indices.length&&indices[0]>previousEnd&&indices.at(-1)-indices[0]+1===indices.length,'PLAN_EVENT_CONTINUITY',`${event.id} 事件制作源须连续、非空且不交错`);previousEnd=indices.at(-1);
  for(const ref of event.sourceRefs)if(required.includes(ref)){need(!owned.has(ref),'PLAN_EVENT_OVERLAP',`${event.id} 源行${ref}重复归属事件`);owned.add(ref);}
  for(const q of source.dialogue)if(q.sourceIds.some(ref=>event.sourceRefs.includes(ref)))need(q.sourceIds.every(ref=>event.sourceRefs.includes(ref)),'PLAN_EVENT_REFS',`${event.id} 原句${q.id}源范围不能分属不同事件`);
  events.set(event.id,event);
 }
 need(required.every(ref=>owned.has(ref)),'PLAN_EVENT_COVERAGE','实际剧情缺少完整事件归属');
 return events;
}

// The lock is saved at intake, before the author receives a mutable production bundle.
export function freezeSource(input){
 need(/^[a-f0-9]{64}$/.test(input?.sourceHash),'SOURCE_LOCK_HASH','冻结原文需要原始剧本sourceHash');
 need(Array.isArray(input.episodes)&&input.episodes.length,'SOURCE_LOCK_SCOPE','冻结原文缺少集范围');
 const numbers=new Set();
 const episodes=input.episodes.map(({number,source}={})=>{
  need(Number.isInteger(number)&&!numbers.has(number),'SOURCE_LOCK_SCOPE','冻结原文集号无效或重复');numbers.add(number);
  const normalized=normalizeSource(source);
  need(!normalized.diagnostics?.length,'SOURCE_LOCK_DIAGNOSTICS',`第${number}集冻结原文仍有归属歧义`);
  return {number,source:structuredClone(source)};
 });
 return {version:'series-source-lock-v1',sourceHash:input.sourceHash,episodes};
}
const sourceLockHash=lock=>createHash('sha256').update(JSON.stringify(lock)).digest('hex');
function checkSourceLock(bundle,lock){
 if(bundle.sourceLockHash===undefined&&lock===undefined)return {mode:'bundle-only',proof:'仅核bundle内部原文与分镜；未与创作前冻结源独立比对'};
 need(/^[a-f0-9]{64}$/.test(bundle.sourceLockHash),'SOURCE_LOCK_HASH','缺少有效冻结原文sourceLockHash');
 need(lock?.version==='series-source-lock-v1','SOURCE_LOCK_REQUIRED','缺少创作前冻结原文；请提供--source-lock');
 need(sourceLockHash(lock)===bundle.sourceLockHash,'SOURCE_LOCK_HASH','冻结原文hash与接入版本不一致');
 need(/^[a-f0-9]{64}$/.test(lock.sourceHash)&&(!bundle.sourceHash||bundle.sourceHash===lock.sourceHash),'SOURCE_LOCK_HASH','冻结原文与原始剧本hash不一致');
 need(Array.isArray(lock.episodes)&&lock.episodes.every(e=>Number.isInteger(e?.number))&&new Set(lock.episodes.map(e=>e.number)).size===lock.episodes.length,'SOURCE_LOCK_SCOPE','冻结原文集范围无效或重复');
 let previous=-1;
 for(const e of bundle.episodes){
  const i=lock.episodes.findIndex(v=>v.number===e.number);
  need(i>previous,'SOURCE_LOCK_SCOPE',`第${e.number}集不在冻结原文范围或源序中`);previous=i;
  need(isDeepStrictEqual(normalizeSource(e.source),normalizeSource(lock.episodes[i].source)),'SOURCE_LOCK_SOURCE',`第${e.number}集source与创作前冻结原文不一致；不可用改源适配改词`);
 }
 return {mode:'locked-source-v1',sourceLockHash:bundle.sourceLockHash,proof:'分镜对照创作前冻结的接入source；接入文本与原件仍须核源'};
}

export function checkBundle(bundle,options={}){
 need(bundle?.version==='series-storyboard-bundle-v1','BUNDLE_VERSION','未知便携输入版本');
 checkGenerationBoundary(bundle);
 const season=checkSeasonBundle(bundle,{scope:options.seasonScope??options.productionScope??options.registry?.productionScope});
 need(bundle.exportProfile===undefined||['series-single-zone-v1','lingmo-v7'].includes(bundle.exportProfile),'EXPORT_PROFILE','未知导出协议，不能默认回退');
 need(bundle.generationTextVersion===undefined||bundle.generationTextVersion==='voice-delivery-v1','GENERATION_TEXT_VERSION','未知生成文本策略');
 if(bundle.sourceLockHash!==undefined)need(bundle.generationTextVersion==='voice-delivery-v1','GENERATION_TEXT_VERSION','新冻结源稿须明确voice-delivery-v1生成文本策略，不回退原始cue自动注入');
 const lingmo=bundle.exportProfile==='lingmo-v7';
 need(typeof bundle.testFixture==='boolean'&&has(bundle.title),'BUNDLE','必须声明标题及 testFixture');
 need(Array.isArray(bundle.unresolved)&&!bundle.unresolved.length,'UNRESOLVED','交付仍有未解决问题');
 need(Array.isArray(bundle.assets),'ASSETS','缺少资产正式名和状态映射');
 const assets=new Map();
 for(const a of bundle.assets){
  need(a&&['CH','SC','PR'].includes(a.type)&&(lingmo?sceneLabel(a.name):cleanName(a.name))&&a.key===a.name+a.type&&has(a.status),'ASSET','资产键、类型、正式名或状态无效');
  need(!assets.has(a.key),'ASSET_DUPLICATE',`重复资产键 ${a.key}`);assets.set(a.key,a);
 }
 if(lingmo)validateImportMappings(bundle.assets);
 need(Array.isArray(bundle.scope)&&bundle.scope.length&&Array.isArray(bundle.episodes),'SCOPE','缺少完整集场范围');
 need(bundle.scope.length===bundle.episodes.length,'EPISODE_COVERAGE','输入集数与全量范围不同');
 const visualBaseline=checkVisualBaselines(bundle);
 const sourceFidelity=checkSourceLock(bundle,options.sourceLock);
 const episodeNumbers=new Set(),stats=[];
 for(const [ei,e] of bundle.episodes.entries()){
  const scope=bundle.scope[ei];
  need(Number.isInteger(e?.number)&&!episodeNumbers.has(e.number)&&e.number===scope?.number,'EPISODE_ORDER','集数缺漏、重复或与源scope顺序不同');episodeNumbers.add(e.number);
  need(Array.isArray(scope.sceneIds)&&scope.sceneIds.length&&scope.sceneIds.every(sceneLabel)&&new Set(scope.sceneIds).size===scope.sceneIds.length,'SCOPE','原场号须为唯一非空单行标签');
  need(Array.isArray(e.scenes)&&isDeepStrictEqual(e.scenes.map(s=>s.id),scope.sceneIds),'SCENE_COVERAGE','场目录未按原集场完整覆盖');
  need(e.settings&&has(e.settings.visualStyle)&&has(e.settings.prohibitions),'SETTINGS','画质及禁止项须由项目明确提供，导出器不补默认画风');
  need(e.plan&&Array.isArray(e.plan.fantasyCandidates)&&e.selection&&Array.isArray(e.selection.selectedIds),'PLAN','缺少统筹候选或采用记录');
  const fantasyMode=e.settings.fantasyMode===undefined?'recommended':e.settings.fantasyMode,candidateIds=e.plan.fantasyCandidates.map(f=>f?.id);
  need(['off','recommended','all','review'].includes(fantasyMode),'FANTASY_MODE','未知幻想模式');
  if(e.settings.fantasyEnabled!==undefined)need(typeof e.settings.fantasyEnabled==='boolean'&&e.settings.fantasyEnabled===(fantasyMode!=='off'),'FANTASY_MODE','旧fantasyEnabled须为布尔值且与正式fantasyMode一致');
  need(candidateIds.every(has)&&new Set(candidateIds).size===candidateIds.length,'FANTASY_CANDIDATE','幻想候选ID缺失或重复');
  need(new Set(e.selection.selectedIds).size===e.selection.selectedIds.length&&e.selection.selectedIds.every(id=>e.plan.fantasyCandidates.some(f=>f.id===id)),'FANTASY_SELECTION','采用幻想重复或不存在');
  if(fantasyMode==='off')need(!e.selection.selectedIds.length&&!list(e.draft?.fantasyRealization).length,'FANTASY_MODE','off不能采用或兑现额外幻想；原文明示的表现要求继续按源保留');
  if(fantasyMode==='all')need(sameSet(e.selection.selectedIds,candidateIds),'FANTASY_MODE','all须采用实际提出的全部候选；逐项选择须另行明确记录review');
  const source=normalizeSource(e.source);
  for(const q of source.dialogue){need(voicePresentations.has(q.channel),'CHANNEL',`原句 ${q.id} 存在不支持的源声道 ${q.channel}，不能默认回退`);need(voiceKinds.has(q.kind),'VOICE_KIND',`原句 ${q.id} 原声类别不受支持，不推断新声型`);}
  if(e.source.dialogue!==undefined){
   const inferred=normalizeSource({...e.source,dialogue:undefined});
   const signature=q=>[q.speaker,q.quote,q.channel,q.sourceIds];
   need(isDeepStrictEqual(source.dialogue.map(signature),inferred.dialogue.map(signature)),'SOURCE_DIALOGUE_COVERAGE','显式对白目录须完整对应原文所有原声，不得借目录删去对白');
   const sourceVoice=q=>[q.cue,q.kind,q.originalChannel,q.performanceCues];
   for(const [i,q] of source.dialogue.entries())need(isDeepStrictEqual(sourceVoice(q),sourceVoice(inferred.dialogue[i])),'SOURCE_DIALOGUE_BINDING',`原句${q.id}显式原声提示、类别或表演位置与源文不一致`);
  }
  need(!source.diagnostics?.length,'SOURCE_DIAGNOSTICS','源对白/动作归属存在未解决歧义');
  const rowScene=new Map(),sceneMap=new Map();let previousRow=-1;
  for(const scene of e.scenes){
   need(validDayInterior(scene.dayInterior),'DAY_INTERIOR',`${scene.id} 日夜内外须逐维明确或待确认`);
   need(Array.isArray(scene.sourceRefs)&&scene.sourceRefs.length,'SCENE_SOURCE',`${scene.id} 缺少原文映射`);
   for(const ref of scene.sourceRefs){const idx=source.sourceRows.findIndex(r=>r.id===ref);need(idx>=0&&!rowScene.has(ref)&&idx>previousRow,'SCENE_SOURCE',`${scene.id} 原文归场未知、重复、跨场或倒序`);rowScene.set(ref,scene.id);previousRow=idx;}
   need(Array.isArray(scene.bindings)&&new Set(scene.bindings).size===scene.bindings.length&&scene.bindings.every(k=>assets.has(k)),'SCENE_BINDINGS',`${scene.id} 场级绑定无效`);sceneMap.set(scene.id,scene);
  }
  need(source.sourceRows.length===rowScene.size,'SCENE_SOURCE','所有原文行须且只须归属一个原场');
  const events=checkedEvents(e.plan,source);
  let previousScene=-1;const used=new Map(e.scenes.map(s=>[s.id,new Set()]));
  for(const u of list(e.draft?.units)){
   const si=e.scenes.findIndex(s=>s.id===u.sceneId);need(si>=0,'UNIT_SCENE',`${u.id} 无法映射原场`);
   need(si>=previousScene,'SCENE_ORDER',`${u.id} 单元原场倒序`);previousScene=si;
   if(u.dayInterior!==undefined)need(validDayInterior(u.dayInterior),'DAY_INTERIOR',`${u.id} 单元日夜内外覆盖无效，不能忽略已给值`);
   need(Array.isArray(u.bindings)&&(lingmo||u.bindings.length<=30)&&new Set(u.bindings).size===u.bindings.length&&u.bindings.every(k=>assets.has(k)),'UNIT_BINDINGS',`${u.id} 缺少显式绑定、重复、未知或旧格式超过30项`);
   if(lingmo)need(Array.isArray(u.imageBindings)&&u.imageBindings.length<=30&&new Set(u.imageBindings).size===u.imageBindings.length&&u.imageBindings.every(key=>u.bindings.includes(key)),'IMAGE_BINDINGS',`${u.id} 必须明确本段图片引用，且无重复、未知或超过30项`);
   const bindings=u.bindings;for(const k of bindings)used.get(u.sceneId).add(k);
   need(sameSet(names(bindings,assets,'SC'),[u.scene]),'UNIT_BINDINGS',`${u.id} 场景名与SC绑定不闭合`);
   need(sameSet(names(bindings,assets,'PR'),list(u.props)),'UNIT_BINDINGS',`${u.id} 道具与PR绑定不闭合`);
   const subjects=list(u.visualSubjects),media=list(u.mediaSubjects),characters=names(bindings,assets,'CH'),participants=list(u.participants),event=events?.get(u.eventId);
   if(u.visualSubjects!==undefined)need(validVisualSubjects(u.visualSubjects),'VISUAL_SUBJECTS',`${u.id} 视觉主体名称、域或唯一性无效`);
   const visual=lingmo?visualBindings(u,subjects,assets):null;
   need(!subjects.some(s=>participants.includes(s.name)),'VISUAL_PRESENCE',`${u.id} 视觉专用主体不能冒充现实出场`);
   if(u.mediaSubjects!==undefined){
    need(Array.isArray(u.mediaSubjects),'MEDIA_SUBJECT',`${u.id} 媒体主体须为数组`);
    const seen=new Set();
    for(const subject of media){
     need(subject&&typeof subject==='object'&&!Array.isArray(subject)&&Object.keys(subject).every(k=>['name','medium','sourceRefs'].includes(k))&&has(subject.name)&&['screen','photo','flashback'].includes(subject.medium),'MEDIA_SUBJECT',`${u.id} 媒体主体字段或域无效`);
     const key=JSON.stringify([subject.name,subject.medium]);
     need(!seen.has(key)&&characters.includes(subject.name)&&Array.isArray(subject.sourceRefs)&&subject.sourceRefs.length&&new Set(subject.sourceRefs).size===subject.sourceRefs.length&&subject.sourceRefs.every(ref=>rowScene.get(ref)===u.sceneId),'MEDIA_SUBJECT',`${u.id} 媒体主体重复、缺少CH绑定或来源不属于本原场`);seen.add(key);
    }
   }
   need(Array.isArray(u.voiceBindings),'VOICE_BINDING',`${u.id} 缺少逐原句人声资产映射`);
   const voiceMap=new Map();
   for(const v of u.voiceBindings){const asset=assets.get(v.assetKey);need(asset?.type==='CH'&&bindings.includes(v.assetKey)&&!voiceMap.has(v.lineId),'VOICE_BINDING',`${u.id} 人声绑定未知、重复、非CH或不属于单元`);voiceMap.set(v.lineId,asset.name);}
   const speakers=new Set(),voiceIds=new Set();
   for(const shot of list(u.shots)){
    const durationTicks=timeTicks(shot.duration,`${shot.id}.duration`,'DURATION',true);let previousVoiceEnd=0;
    for(const ref of list(shot.sourceRefs))need(rowScene.get(ref)===u.sceneId,'UNIT_SOURCE_SCENE',`${shot.id} 源文引用越过所标原场`);
    for(const p of list(shot.dialogueParts)){
     const start=timeTicks(p.voiceStart,`${shot.id}/${p.lineId}.voiceStart`,'VOICE_WINDOW'),end=timeTicks(p.voiceEnd,`${shot.id}/${p.lineId}.voiceEnd`,'VOICE_WINDOW');
     need(end>start&&end<=durationTicks&&start>=previousVoiceEnd,'VOICE_WINDOW',`${shot.id} 原句${p.lineId}发声窗越界或重叠`);previousVoiceEnd=end;
     const q=source.dialogue.find(q=>q.id===p.lineId);
     if(q){
      need(q.sourceIds.every(ref=>rowScene.get(ref)===u.sceneId),'DIALOGUE_SOURCE_SCENE',`${shot.id} 原句${q.id}发声位置越过源文所属原场`);
      if(event)need(q.sourceIds.every(ref=>event.sourceRefs.includes(ref)),'DIALOGUE_EVENT',`${shot.id} 原句${q.id}不属于单元事件${event.id}`);
      need(voicePresentations.get(q.channel).has(p.presentation),'CHANNEL',`${shot.id} 原句 ${q.id} 的${q.channel}不允许呈现为${p.presentation}`);
     }
     if(q&&q.channel!=='系统音'){need(voiceMap.has(q.id),'VOICE_BINDING',`${u.id} 原句 ${q.id} 缺少明确资产绑定`);speakers.add(voiceMap.get(q.id));voiceIds.add(q.id);}
    }
   }
   need(sameSet([...voiceMap.keys()],[...voiceIds]),'VOICE_BINDING',`${u.id} 人声映射必须恰好对应本单元原句`);
   const visuals=lingmo?visual.filter(v=>v.assetKey!==null).map(v=>assets.get(v.assetKey).name):subjects.map(s=>s.name);
   const expected=[...new Set([...participants,...visuals,...media.map(s=>s.name),...speakers])];
   need(sameSet(characters,expected),'UNIT_BINDINGS',`${u.id} 人物、声源、视觉专用主体与CH绑定不闭合；不凭名字猜键`);
   if(lingmo){
    const visible=new Set([...participants,...media.map(s=>s.name),...visuals]);
    need(u.imageBindings.every(key=>assets.get(key).type!=='CH'||visible.has(assets.get(key).name)),'IMAGE_BINDINGS',`${u.id} 纯声源不能自动成为图片引用；须有明确现实、媒体或视觉主体用途`);
   }
   const totalTicks=list(u.shots).reduce((v,s)=>v+timeTicks(s.duration,`${s.id}.duration`,'DURATION',true),0);
   if(lingmo){
    const cap=[15,30].includes(e.settings.maxUnitSeconds)?e.settings.maxUnitSeconds:30;
    need(Number.isSafeInteger(totalTicks)&&totalTicks>0&&totalTicks<=cap*SECOND_TICKS,'UNIT_RANGE',`${u.id} 实际镜长须大于0且不超过${cap}秒；仅外层容器向上取整并至少4秒`);
   }else{
    need(Number.isSafeInteger(totalTicks)&&totalTicks%SECOND_TICKS===0,'UNIT_INTEGER',`${u.id} 合计镜长须实际为整数，返回局部处理`);
    need(totalTicks>=4*SECOND_TICKS&&totalTicks<=30*SECOND_TICKS,'UNIT_RANGE',`${u.id} 片长须4至30秒`);
   }
   if(u.segments!==undefined){
    need(Array.isArray(u.segments)&&u.segments.length,'SEGMENTS',`${u.id} 原分段无效`);
    for(const s of u.segments)need(s&&['title','lighting','initialState','exitState'].every(k=>has(s[k]))&&Array.isArray(s.shotIds)&&s.shotIds.length,'SEGMENTS',`${u.id} 原分段字段缺失或未归属任何镜头`);
    need(isDeepStrictEqual(u.segments.flatMap(s=>s.shotIds),list(u.shots).map(s=>s.id)),'SEGMENTS',`${u.id} 原分段镜号必须完整同序`);
   }
  }
  for(const scene of e.scenes)need(used.get(scene.id).size>0&&sameSet(scene.bindings,[...used.get(scene.id)]),'SCENE_BINDINGS',`${scene.id} 场级三行须精确等于单元实际引用并集`);
  const result=validateDraft({draft:e.draft,source,settings:e.settings,selection:e.selection,plan:e.plan,unitDirectionVersion:e.unitDirectionVersion});
  if(result.errors.length)fail(result.errors[0].code,JSON.stringify(result.errors));
  stats.push({number:e.number,...result.stats,warnings:result.warnings});
 }
 const productionBindings=bundle.sourceHash!==undefined||bundle.assetRegistry!==undefined||bundle.assets.some(a=>a.assetId!==undefined||a.assetRevision!==undefined);
 const assetRegistry=(!bundle.testFixture||productionBindings||options.registry)?checkAssetBindings(bundle,options.registry):undefined;
 return {ok:true,testFixture:bundle.testFixture,exportProfile:bundle.exportProfile??'series-single-zone-v1',episodes:stats,sourceFidelity,visualBaseline,...(!season.legacy?{season}:{}),...(assetRegistry?{assetRegistry}:{}),proof:'结构及源引用检查；冻结源证据见sourceFidelity；不证明画面语义、素材图片、平台导入或视频效果'};
}

// The projection contains only delivered, visible fields. Internal source/decision ledgers stay in the bundle.
export function projectBundle(bundle,options={}){
 checkBundle(bundle,options);if(bundle.exportProfile==='lingmo-v7')return projectLingmoV7(bundle);
 const assets=new Map(bundle.assets.map(a=>[a.key,a]));
 const ordered=keys=>[...keys].sort((a,b)=>['SC','CH','PR'].indexOf(assets.get(a).type)-['SC','CH','PR'].indexOf(assets.get(b).type));
 const joined=(keys,type)=>names(ordered(keys),assets,type).join('、')||'无';
 const value=(label,text)=>[label,text];
 return bundle.episodes.map(e=>{
  const source=normalizeSource(e.source),quotes=new Map(source.dialogue.map(q=>[q.id,q]));
  return {number:e.number,scenes:e.scenes.map(scene=>({id:scene.id,assets:[joined(scene.bindings,'SC'),joined(scene.bindings,'CH'),joined(scene.bindings,'PR')],units:e.draft.units.filter(u=>u.sceneId===scene.id).map((u,index)=>{
   const opening=u.segments?.[0]||u;let cursor=0;
   const header=[value('引用',ordered(u.bindings).map(k=>`${assets.get(k).name}是@[${k}]`).join('，')+'。'),value('画质说明',e.settings.visualStyle),value('光影效果',opening.lighting),value('开场站位',opening.initialState),value('片段标题',u.title)];
   if(has(u.unitDirection))header.push(value('演出总指令',u.unitDirection));
   const shots=u.shots.map(s=>{
    const start=cursor;cursor+=timeTicks(s.duration,`${s.id}.duration`,'DURATION',true);const fields=[];
    const segment=u.segments?.find(segment=>segment.shotIds[0]===s.id);
    if(segment)for(const [label,key] of [['分段标题','title'],['分段演出','unitDirection'],['分段光影','lighting'],['分段入口','initialState'],['分段出口','exitState']])if(has(segment[key]))fields.push(value(label,segment[key]));
    fields.push(value('内容',s.picture));
    for(const p of s.dialogueParts){const q=quotes.get(p.lineId);fields.push(value('原声类别',`${q.speaker}：${voiceKinds.get(q.kind)}`));if(bundle.generationTextVersion!=='voice-delivery-v1'&&has(q.cue))fields.push(value('原声提示',`${q.speaker}：${q.cue}`));fields.push(value('台词',`${q.speaker}（${q.channel}｜${p.presentation}｜本条${timeText((start+timeTicks(p.voiceStart,`${s.id}/${p.lineId}.voiceStart`))/SECOND_TICKS)}-${timeText((start+timeTicks(p.voiceEnd,`${s.id}/${p.lineId}.voiceEnd`))/SECOND_TICKS)}秒连续原声｜演法：${p.delivery}）：${p.text}`));}
    fields.push(value('运镜',s.camera),value('镜头光影',s.lighting),value('镜头衔接',s.transition),value('镜内声音',s.sound));
    return {start:start/SECOND_TICKS,end:cursor/SECOND_TICKS,fields};
   });
   return {number:index+1,duration:cursor/SECOND_TICKS,dayInterior:u.dayInterior??scene.dayInterior,header,shots,footer:[value('结束状态',u.exitState),value('声音',u.shots.map((s,i)=>`分镜${i+1}：${s.sound}`).join('\n')),value('禁止',e.settings.prohibitions)]};
  })}))};
 });
}

const fieldLines=([label,text])=>`${label}：${text.replaceAll('\n','\n  ')}`;
export function renderProjection(projected){
 if(projected?.profile==='lingmo-v7')return renderLingmoV7(projected);
 const lines=['**【标准化剧本】**'];
 for(const e of projected){lines.push(`第${e.number}集`);for(const scene of e.scenes){lines.push(scene.id,...['场景','人物','道具'].map((k,i)=>`${k}：${scene.assets[i]}`));for(const u of scene.units){lines.push(`片段${u.number}`,`时长：${u.duration}秒`,u.dayInterior,'片段正文：',...u.header.map(fieldLines));for(const s of u.shots)lines.push(`${timeText(s.start)}-${timeText(s.end)}秒：`,...s.fields.map(fieldLines));lines.push(...u.footer.map(fieldLines));}}}
 return lines.join('\n')+'\n';
}

// Independent grammar reader. It never calls renderProjection or accepts an embedded JSON/hash as proof.
export function parseSingleZone(markdown){
 const lines=markdown.split('\n');need(lines.pop()==='','VERIFY_SYNTAX','文件须为LF并末尾换行');let pos=0;
 const take=()=>lines[pos++];const peek=()=>lines[pos];const expect=s=>need(take()===s,'VERIFY_SYNTAX',`第${pos}行应为 ${s}`);
 const sceneStart=()=>typeof peek()==='string'&&['场景：','人物：','道具：'].every((prefix,i)=>lines[pos+i+1]?.startsWith(prefix));
 const readFields=()=>{const out=[];while(pos<lines.length&&!sceneStart()){const match=/^([^：\n]+)：(.*)$/.exec(peek());if(!match)break;const label=match[1];if(!['引用','画质说明','光影效果','开场站位','片段标题','演出总指令','分段标题','分段演出','分段光影','分段入口','分段出口','内容','原声类别','原声提示','台词','运镜','镜头光影','镜头衔接','镜内声音','结束状态','声音','禁止'].includes(label))break;take();let text=match[2];while(peek()?.startsWith('  ')&&!sceneStart())text+='\n'+take().slice(2);out.push([label,text]);}return out;};
 expect('**【标准化剧本】**');const episodes=[];
 while(pos<lines.length){const em=/^第(-?\d+)集$/.exec(take());need(em,'VERIFY_SYNTAX',`第${pos}行缺少集标题`);const e={number:Number(em[1]),scenes:[]};episodes.push(e);
  while(pos<lines.length&&(!/^第-?\d+集$/.test(peek())||sceneStart())){const scene={id:take(),assets:[],units:[]};for(const label of ['场景','人物','道具']){const prefix=label+'：',line=take();need(line?.startsWith(prefix),'VERIFY_SYNTAX',`第${pos}行缺少场级${label}`);scene.assets.push(line.slice(prefix.length));}e.scenes.push(scene);
   while(/^片段\d+$/.test(peek())&&!sceneStart()){const number=Number(take().slice(2)),dm=/^时长：(\d+)秒$/.exec(take());need(dm,'VERIFY_SYNTAX','片长必须为整数');const dayInterior=take();expect('片段正文：');const header=readFields(),shots=[];
    while(/^\d+(?:\.\d+)?-\d+(?:\.\d+)?秒：$/.test(peek())&&!sceneStart()){const match=/^(\d+(?:\.\d+)?)-(\d+(?:\.\d+)?)秒：$/.exec(take());shots.push({start:Number(match[1]),end:Number(match[2]),fields:readFields()});}
    // Footer follows the last shot fields; split it using an explicit first footer label.
    const last=shots.at(-1);need(last,'VERIFY_SYNTAX','片段缺少镜窗');const at=last.fields.findIndex(([label])=>label==='结束状态');need(at>=0,'VERIFY_SYNTAX','片段缺少结束状态');const footer=last.fields.splice(at);
    scene.units.push({number,duration:Number(dm[1]),dayInterior,header,shots,footer});
   }
  }
 }
 return episodes;
}

export function verifyMarkdown(bundle,markdown,options={}){
 const expected=projectBundle(bundle,options);
 if(bundle.exportProfile==='lingmo-v7')verifyLingmoProjection(expected,markdown);
 else need(isDeepStrictEqual(parseSingleZone(markdown),expected),'VERIFY_MISMATCH','反解后的集场、引用、正文、镜窗、原声、分段或禁止字段与采用稿不一致');
 const season=checkSeasonBundle(bundle,{scope:options.seasonScope??options.productionScope??options.registry?.productionScope});
 return {ok:true,...(bundle.exportProfile==='lingmo-v7'?{fieldCapacity:inspectFieldBudget(markdown)}:{}),testFixture:bundle.testFixture,exportProfile:bundle.exportProfile??'series-single-zone-v1',sourceFidelity:checkSourceLock(bundle,options.sourceLock),visualBaseline:checkVisualBaselines(bundle),...(!season.legacy?{season}:{}),proof:'独立反解全量可见字段一致；冻结源证据见sourceFidelity；不代表内容审核、平台导入或视频效果'};
}

async function main(){
 const [command,...args]=process.argv.slice(2),options={};need(['inspect-source','freeze-source','check','export','verify'].includes(command),'USAGE','node scripts/storyboard.mjs inspect-source --input SOURCE.json; freeze-source --input INTAKE.json --output LOCK.json; check|export|verify --input BUNDLE --source-lock LOCK.json --registry DIR [--output FILE] [--assets-index FILE]');
 need(args.length%2===0,'USAGE','参数须成对出现');for(let i=0;i<args.length;i+=2){need(['--input','--output','--registry','--assets-index','--source-lock','--season-scope'].includes(args[i])&&!Object.hasOwn(options,args[i])&&has(args[i+1]),'USAGE','未知、缺值或重复参数');options[args[i]]=args[i+1];}
 need(options['--input'],'USAGE','缺少 --input');const bytes=await readFile(options['--input']),bundle=JSON.parse(bytes);
 if(command==='inspect-source'){need(!options['--output']&&!options['--registry']&&!options['--assets-index']&&!options['--source-lock']&&!options['--season-scope'],'USAGE','inspect-source只读源，不接受输出、锁、季范围或总库参数');console.log(JSON.stringify(normalizeSource(bundle)));return;}
 if(command==='freeze-source'){
  need(options['--output']&&!options['--registry']&&!options['--assets-index']&&!options['--source-lock']&&!options['--season-scope'],'USAGE','freeze-source只接受接入输入及独立输出');
  const lock=freezeSource(bundle),file=resolve(options['--output']);
  await mkdir(dirname(file),{recursive:true});await writeFile(file,JSON.stringify(lock,null,2)+'\n',{flag:'wx'});
  console.log(JSON.stringify({ok:true,file,sourceLockHash:sourceLockHash(lock),episodes:lock.episodes.map(e=>e.number)}));return;
 }
 const context={...(options['--registry']?{registry:(await readRegistry(options['--registry'])).registry}:{}),...(options['--source-lock']?{sourceLock:JSON.parse(await readFile(options['--source-lock'],'utf8'))}:{}),...(options['--season-scope']?{seasonScope:JSON.parse(await readFile(options['--season-scope'],'utf8'))}:{})};
 if(command==='check'){need(!options['--assets-index'],'USAGE','check不读素材目录；目录配套检查用verify');console.log(JSON.stringify(checkBundle(bundle,context)));return;}
 need(options['--output'],'USAGE','缺少 --output');const file=options['--output'];
 const lingmo=bundle.exportProfile==='lingmo-v7',index=lingmo?(options['--assets-index']??`${file}.assets.json`):null;
 need(lingmo||!options['--assets-index'],'USAGE','旧导出协议不接受v7素材目录');
 if(index)need(resolve(index)!==resolve(file)&&resolve(index)!==resolve(options['--input']),'USAGE','MD、输入及素材目录须为不同文件');
 const bundleHash=createHash('sha256').update(bytes).digest('hex');
 if(command==='verify'){
  const markdown=await readFile(file,'utf8'),result=verifyMarkdown(bundle,markdown,context);
  if(index){const expected=buildLingmoCatalog(bundle,projectBundle(bundle,context),markdown,bundleHash),actual=JSON.parse(await readFile(index,'utf8'));need(isDeepStrictEqual(actual,expected),'VERIFY_ASSET_INDEX','素材目录与当前采用稿、MD、逐段图片引用或命名映射不一致');result.assetsIndex=resolve(index);result.imageAssets=actual.assets.length;}
  console.log(JSON.stringify(result));return;
 }
 const projected=projectBundle(bundle,context),markdown=renderProjection(projected);verifyMarkdown(bundle,markdown,context);
 const outputs=[[resolve(file),markdown]];
 if(index)outputs.push([resolve(index),JSON.stringify(buildLingmoCatalog(bundle,projected,markdown,bundleHash),null,2)+'\n']);
 const created=[];
 try{for(const [path,content] of outputs){await mkdir(dirname(path),{recursive:true});await writeFile(path,content,{flag:'wx'});created.push(path);}}
 catch(error){for(const path of created)await unlink(path).catch(()=>{});throw error;}
 console.log(JSON.stringify({ok:true,...(lingmo?{fieldCapacity:inspectFieldBudget(markdown)}:{}),file:resolve(file),...(index?{assetsIndex:resolve(index)}:{}),testFixture:bundle.testFixture,sourceFidelity:checkSourceLock(bundle,context.sourceLock),visualBaseline:checkVisualBaselines(bundle),proof:'结构导出及独立回读一致；冻结源证据见sourceFidelity；v7目录由同一映射生成'}));
}
if(process.argv[1]&&realpathSync(fileURLToPath(import.meta.url))===realpathSync(process.argv[1]))main().catch(error=>{console.error(`${error.code||'ERROR'}: ${error.message}`);process.exitCode=1;});
