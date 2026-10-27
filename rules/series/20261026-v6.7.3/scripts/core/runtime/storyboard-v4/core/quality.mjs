import {VERSION,fail} from './shared.mjs';
import {resolveSourceRef} from './source.mjs';
import {UNIT_PACKING_POLICY,packEventUnits} from './event-unit-packing.mjs';
import {UNIT_DIRECTION_VERSION} from './output-schema.mjs';
import {SUPPORTED_SEEDANCE_EXPORT_VERSIONS} from './visual-style.mjs';
import {FANTASY_TIMING_VERSION,fantasyTiming,normalizeFantasyTiming} from './fantasy-timing.mjs';
import {validVisualSubjects} from './visual-subjects.mjs';
import {resolvedDialogue} from './source-resolution.mjs';

const has=x=>typeof x==='string'&&!!x.trim();
const issue=(code,coordinate,detail)=>({code,coordinate,detail});
const num=x=>Number.isFinite(x)?Number(x.toFixed(3)):x;
const canonical=(r,source)=>r.id||`${source.key}:${r.sourceId||`L${String(r.line).padStart(4,'0')}`}`;
function resolvedId(source,ref,allowContext=false){
 try{return canonical(resolveSourceRef(source,ref,{allowContext}),source);}catch{return null;}
}
const synonyms={'画外→画内':'画外转画内','画外到画内':'画外转画内','画外切画内':'画外转画内','画内→画外':'画内转画外','画内到画外':'画内转画外','画内切画外':'画内转画外'};
const presentations=new Set(['画内','画外','VO','OS','系统音','画外转画内','画内转画外']);

export function normalizeDraft(raw,source,options={}){
 const draft=structuredClone(raw),changes=[];
 const change=(obj,key,value,coordinate)=>{if(value!==undefined&&obj[key]!==value){changes.push({coordinate,field:key,from:obj[key],to:value});obj[key]=value;}};
 const contextIds=new Set((source?.contextRows||[]).map(r=>canonical(r,source)));
 for(const u of Array.isArray(draft?.units)?draft.units:[])for(const s of Array.isArray(u?.shots)?u.shots:[]){
  if(!s||typeof s!=='object')continue;
  if(Array.isArray(s.sourceRefs)){
   const kept=[];
   for(const [i,ref] of s.sourceRefs.entries()){
    const sourceId=resolvedId(source,ref),contextId=sourceId?null:resolvedId(source,ref,true);
    if(contextId&&contextIds.has(contextId)&&(s.contextRefs===undefined||Array.isArray(s.contextRefs))){
     s.contextRefs??=[];
     if(!s.contextRefs.includes(contextId))s.contextRefs.push(contextId);
     changes.push({coordinate:s.id,field:`sourceRefs[${i}]`,from:ref,to:contextId,target:'contextRefs',action:'move'});
     continue;
    }
    if(sourceId&&sourceId!==ref){changes.push({coordinate:s.id,field:`sourceRefs[${i}]`,from:ref,to:sourceId});kept.push(sourceId);}
    else kept.push(ref);
   }
   s.sourceRefs=kept;
  }
  if(Array.isArray(s.contextRefs))s.contextRefs=s.contextRefs.map((ref,i)=>{const id=resolvedId(source,ref,true);if(id&&id!==ref){changes.push({coordinate:s.id,field:`contextRefs[${i}]`,from:ref,to:id});return id;}return ref;});
  for(const p of Array.isArray(s.dialogueParts)?s.dialogueParts:[])if(p&&typeof p==='object')change(p,'presentation',synonyms[p.presentation],s.id);
 }
 for(const f of Array.isArray(draft?.fantasyRealization)?draft.fantasyRealization:[])if(f&&typeof f==='object')change(f,'status',{'已兑现':'realized',implemented:'realized'}[f.status],f.candidateId);
 changes.push(...normalizeFantasyTiming(draft,options.fantasyTimingVersion));
 changes.push(...packEventUnits(draft,options));
 return {draft,changes};
}

export function validateDraft({draft,source,settings={},selection,plan,unitDirectionVersion}={}){
 settings=settings&&typeof settings==='object'?settings:{};
 const errors=[],warnings=[],add=(code,coord,detail)=>errors.push(issue(code,coord,detail));
 const units=Array.isArray(draft?.units)?draft.units:[],qs=source?.dialogue||[],qmap=new Map(qs.map((q,i)=>[q.id,{...q,index:i}]));
  const rows=new Map((source?.sourceRows||[]).map(r=>[canonical(r,source),r]));
 const covered=new Set(),unitIds=new Set(),shotIds=new Set(),owner=new Map(),assembled=new Map();let previous=-1,partCount=0,seconds=0;
 if(draft?.schemaVersion!==VERSION)add('SCHEMA','schemaVersion',`应为${VERSION}`);
 if(!units.length)add('UNITS','units','缺少完整单元');
 if(draft?.unresolved!==undefined&&(!Array.isArray(draft.unresolved)||draft.unresolved.length))add('UNRESOLVED','unresolved','整编仍有未解决问题，不能作为可交稿');
 const cap=settings.maxUnitSeconds===undefined?30:settings.maxUnitSeconds;
 const events=plan?.unitPackingPolicy===UNIT_PACKING_POLICY?new Map((plan.events||[]).map(e=>[e.id,e])):null;
 if(![15,30].includes(cap))add('SETTINGS','maxUnitSeconds','单元时长上限须为15或30秒');
 if(settings.maxUnits!=null&&units.length>settings.maxUnits)add('MAX_UNITS','units','超过单元数上限');
 for(const u of units){
  const uid=u?.id||'unit';if(!has(u?.id)||unitIds.has(u.id))add('UNIT_ID',uid,'单元ID缺失或重复');unitIds.add(u?.id);
  const event=events?.get(u?.eventId);
  if(events&&!event)add('EVENT_ID',uid,'单元未归属本次明确完整事件');
  if(events&&(!['episode_start','continuous','capacity','new_event','time_space_jump'].includes(u?.boundaryBefore?.kind)||!has(u?.boundaryBefore?.reason)))add('EVENT_BOUNDARY',uid,'单元边界或依据缺失');
  for(const k of ['title','scene','lighting','initialState','exitState'])if(!has(u?.[k]))add('FIELD',uid,`单元${k}缺失`);
  if(unitDirectionVersion===UNIT_DIRECTION_VERSION){
   if(!has(u?.unitDirection))add('UNIT_DIRECTION',uid,'单元专属演出总指令缺失');
   for(const [i,segment] of (Array.isArray(u?.segments)?u.segments:[]).entries())if(!has(segment?.unitDirection))add('UNIT_DIRECTION',`${uid}.segments[${i}]`,'合并前分段的专属演出总指令缺失');
  }
  for(const k of ['props','participants'])if(!Array.isArray(u?.[k])||u[k].some(x=>!has(x)))add('FIELD',uid,`单元${k}格式错误`);
  if(u&&Object.hasOwn(u,'visualSubjects')&&!validVisualSubjects(u.visualSubjects))add('VISUAL_SUBJECTS',uid,'主观或未来图示主体清单格式、类型或唯一性无效；不当作现实出场或觉醒事实');
  if(!u?.intent||!['subject','change','landing'].every(k=>has(u.intent[k]))||!Array.isArray(u.intent.retain)||!u.intent.retain.length||u.intent.retain.some(x=>!has(x)))add('INTENT',uid,'情绪主体、变化、落点及保留功能不完整');
  if(!Array.isArray(u?.shots)||!u.shots.length){add('SHOTS',uid,'缺少镜头');continue;}
  let duration=0;
  for(const s of u.shots){
   const sid=s?.id||uid;if(!has(s?.id)||shotIds.has(s.id))add('SHOT_ID',sid,'镜头ID缺失或重复');shotIds.add(s?.id);owner.set(s?.id,uid);
   if(!Number.isFinite(s?.duration)||s.duration<=0)add('DURATION',sid,'镜长须为正数');else duration+=s.duration;
   for(const k of ['camera','picture','lighting','sound','transition'])if(!has(s?.[k]))add('FIELD',sid,`镜头${k}缺失`);
   if(has(s?.camera)&&/(?:先[^。；;]{0,35}再切(?:成|至|到)|同一镜内[^。；;]{0,35}(?:随即|然后)[^。；;]{0,10}切|连续摇焦[^。；;]{0,15}切|(?:随即|然后)切同一镜内)/.test(s.camera))warnings.push(issue('CAMERA_CUT',sid,'单条镜头疑似另起硬切，请核对是否需拆镜计时'));
   if(!Array.isArray(s?.sourceRefs))add('SOURCE_REF',sid,'缺少源引用数组');
   else for(const ref of s.sourceRefs){const id=resolvedId(source,ref);if(!id||!rows.has(id))add('SOURCE_REF',sid,`未知或有歧义的源引用${ref}`);else {covered.add(id);if(event&&!event.sourceRefs.includes(id))add('EVENT_SOURCE',sid,'镜头制作源越出所标完整事件');}}
   if(s?.contextRefs!==undefined){
    if(!Array.isArray(s.contextRefs))add('CONTEXT_REF',sid,'前情引用须为数组');
    else for(const ref of s.contextRefs){const id=resolvedId(source,ref,true);if(!id)add('CONTEXT_REF',sid,`未知或有歧义的前情引用${ref}`);}
   }
   if(!Array.isArray(s?.dialogueParts)){add('FIELD',sid,'缺少dialogueParts数组');continue;}
   let voiceEnd=0;
   for(const p of s.dialogueParts){
    partCount++;const q=qmap.get(p?.lineId);if(!q){add('DIALOGUE_REF',sid,`未知原句${p?.lineId}`);continue;}
    if(q.index<previous)add('DIALOGUE_ORDER',sid,`原句${p.lineId}倒序`);previous=q.index;
    if(!has(p.text))add('DIALOGUE_CONTENT',sid,'台词片段为空');assembled.set(q.id,(assembled.get(q.id)||'')+(p.text||''));
    if(!Number.isFinite(p.voiceStart)||!Number.isFinite(p.voiceEnd)||p.voiceStart<0||p.voiceEnd<=p.voiceStart||p.voiceEnd>s.duration+1e-6||p.voiceStart<voiceEnd-1e-6)add('VOICE_WINDOW',sid,`原句${p.lineId}发声窗越界或叠话未声明`);voiceEnd=p.voiceEnd;
    if(!presentations.has(p.presentation)||!has(p.delivery))add('DIALOGUE_PRESENTATION',sid,`原句${p.lineId}呈现或演法缺失`);
    if(['VO','OS'].includes(q.channel)&&p.presentation!==q.channel)add('CHANNEL',sid,`原句${p.lineId}的${q.channel}声道被改变`);
    if(q.channel==='现场对白'&&['VO','OS'].includes(p.presentation))add('CHANNEL',sid,`原句${p.lineId}现场声被改变`);
    if(q.channel==='系统音'&&!['画外','系统音'].includes(p.presentation))add('CHANNEL',sid,`原句${p.lineId}系统音被改变`);
   }
  }
  seconds+=duration;if(duration>([15,30].includes(cap)?cap:30)+1e-6)add('UNIT_DURATION',uid,`单元${num(duration)}秒超过${[15,30].includes(cap)?cap:30}秒`);
 }
 for(const q of qs)if(assembled.get(q.id)!==q.quote)add('DIALOGUE_CONTENT',q.id,'逐字拼接与原话不一致（含缺漏或重复）');
 for(const r of source?.sourceRows||[])if(has(r.text)&&!['blank','punctuation','metadata'].includes(r.kind)&&!/^\s*[！!？?。，,.、…；;：:]+\s*$/.test(r.text)&&(r.kind||!/^\s*(?:场景[：:]|地点[：:]|时间[：:]|出场人物[：:]|第.{0,12}集)/.test(r.text))&&!covered.has(canonical(r,source)))add('SOURCE_COVERAGE',canonical(r,source),'源剧情行未关联镜头；关联不证明画面语义正确');
 for(const f of source?.entryFacts||[])for(const ref of f.sourceRefs||[])if(!resolvedId(source,ref,true))add('ENTRY_REF',f.id||'entryFact',`关键前情出处无效：${ref}`);
 const reality=Array.isArray(draft?.fantasyRealization)?draft.fantasyRealization:[];
 if(!Array.isArray(draft?.fantasyRealization))add('FANTASY_FIELD','fantasyRealization','缺少幻想兑现记录数组');
 const selected=new Set(selection?.selectedIds||[]),candidates=new Set((plan?.fantasyCandidates||[]).map(x=>x.id)),seen=new Set();
 for(const [index,f] of reality.entries()){
  if(!f||typeof f!=='object'){add('FANTASY_FIELD',`fantasyRealization[${index}]`,'幻想兑现记录须为对象');continue;}
  const id=f?.candidateId||'fantasy';if(seen.has(id))add('FANTASY_DUPLICATE',id,'候选兑现记录重复');seen.add(id);
  if(!candidates.has(id))add('FANTASY_UNKNOWN',id,'未知幻想候选');
  if(selection&&!selected.has(id))add('FANTASY_UNSELECTED',id,'未选候选不得实施');
  if(!['realized','not_realized'].includes(f.status))add('FANTASY_STATUS',id,'兑现状态无效');
  if(Object.hasOwn(f,'screenSpans')){
   const timing=fantasyTiming(f,draft);
   if(!timing||f.timingVersion!==FANTASY_TIMING_VERSION||JSON.stringify(f.shotIds)!==JSON.stringify(timing.shotIds)||f.screenSeconds!==timing.screenSeconds)add('FANTASY_SPAN',id,'幻想镜内范围、顺序、归属或派生占时不一致；不猜入出或截断镜长');
  }
  if(f.status!=='realized')continue;
  if(!Array.isArray(f.unitIds)||!f.unitIds.length||new Set(f.unitIds).size!==f.unitIds.length||f.unitIds.some(uid=>!unitIds.has(uid)))add('FANTASY_UNIT',id,'缺少有效单元归属或归属重复');
  const shots=units.flatMap(u=>u?.shots||[]),linked=Array.isArray(f.shotIds)?f.shotIds.map(sid=>shots.find(s=>s?.id===sid)):[];
  if(!Array.isArray(f.shotIds)||!f.shotIds.length||new Set(f.shotIds).size!==f.shotIds.length||f.shotIds.some((sid,i)=>!shotIds.has(sid)||!Array.isArray(f.unitIds)||!f.unitIds.includes(owner.get(sid))||!has(linked[i]?.picture)))add('FANTASY_SHOT',id,'缺少有效镜号归属、镜头重复或画面正文');
  if(linked.length&&linked.every(s=>Number.isFinite(s?.duration))&&Number.isFinite(f.screenSeconds)&&f.screenSeconds>linked.reduce((n,s)=>n+s.duration,0)+1e-6)add('FANTASY_DURATION',id,'幻想屏幕时长超过关联镜头合计镜长');
  if(!Number.isFinite(f.screenSeconds)||f.screenSeconds<=0||!['entry','exit','domain','preservedFunction'].every(k=>has(f[k])))add('FANTASY_DETAIL',id,'屏幕时长、入出与功能不完整');
 }
 for(const id of selected)if(!reality.some(f=>f?.candidateId===id&&f.status==='realized'))add('FANTASY_MISSING',id,'已选幻想尚无兑现记录');
 return {errors,warnings,stats:{units:units.length,shots:shotIds.size,dialogueParts:partCount,seconds:num(seconds),sourceRowsCovered:covered.size,sourceRowsTotal:source?.sourceRows?.length||0,selectedFantasy:selected.size,realizedFantasy:reality.filter(f=>f?.status==='realized').length}};
}

function enclosingSpeechQuotes(text){
 if(typeof text!=='string'||!text.startsWith('“')||!text.endsWith('”'))return false;
 let depth=0;
 for(let i=0;i<text.length;i++){
  if(text[i]==='“')depth++;
  else if(text[i]==='”'){depth--;if(depth<0||depth===0&&i!==text.length-1)return false;}
 }
 return depth===0;
}
function sourceQuoteEdges(draft,quotes){
 const byLine=new Map(),first=new Set(),last=new Set();
 for(const unit of draft?.units||[])for(const shot of unit.shots||[])for(const part of shot.dialogueParts||[]){
  if(!byLine.has(part.lineId))byLine.set(part.lineId,[]);byLine.get(part.lineId).push(part);
 }
 for(const [lineId,parts] of byLine){
  const quote=quotes.get(lineId)?.quote;
  if(!enclosingSpeechQuotes(quote)||parts.map(p=>p.text).join('')!==quote)continue;
  const nonempty=parts.filter(p=>typeof p.text==='string'&&p.text.length);
  if(nonempty.length){first.add(nonempty[0]);last.add(nonempty.at(-1));}
 }
 return {first,last};
}

export function renderDraft({draft,source,settings={},seedanceExportVersion,sourceResolutions=[]}={}){
 if(seedanceExportVersion!==undefined&&!SUPPORTED_SEEDANCE_EXPORT_VERSIONS.includes(seedanceExportVersion))fail('EXPORT_VERSION','未知视频执行稿导出版本');
 const execution=seedanceExportVersion!==undefined;
 const unitTimeline=seedanceExportVersion==='seedance-execution-v3';
 const quotes=new Map((source?resolvedDialogue(source,sourceResolutions):[]).map(q=>[q.id,q])),sections=[];
 // V2 and later only omit redundant display delimiters at verified source edges; stored text is untouched.
 const quoteEdges=seedanceExportVersion==='seedance-execution-v2'||unitTimeline?sourceQuoteEdges(draft,quotes):null;
 const style=has(settings?.visualStyle)?settings.visualStyle:'3D国漫横屏短剧，16:9；角色、场景与光影连续，表演和空间关系清晰。';
 for(const u of draft?.units||[]){
  let t=0;const total=(u.shots||[]).reduce((n,s)=>n+(Number.isFinite(s.duration)?s.duration:0),0);
  const records=Array.isArray(draft?.fantasyRealization)?draft.fantasyRealization:[];
  const realized=records.some(f=>f?.status==='realized'&&Array.isArray(f.unitIds)&&f.unitIds.includes(u.id));
  const active=settings?.fantasyMode==='off'?false:records.length?realized:settings?.fantasyEnabled;
  const displayHeader=value=>typeof value!=='string'?value:value.replace(/【([^】]*)】/g,(_match,anchor)=>{
   for(const [i,shot] of (u.shots||[]).entries()){
    const escaped=shot.id?.replace(/[.*+?^${}()|[\]\\]/g,'\\$&');
    if(escaped)anchor=anchor.replace(new RegExp(`(^|[—、至])${escaped}(?=$|[—、至起])`,'g'),`$1分镜${i+1}`);
   }
   return `【${anchor}】`;
  });
  // Project decided execution fields directly; never scrub arbitrary source or saved prose.
  const common=execution
   ?[style,has(u.unitDirection)?displayHeader(u.unitDirection):null].filter(has).join('\n')
   :has(u.unitDirection)
   ?`${style}\n${active?'已选幻想只按本单元画面正文呈现':'本单元不加入额外幻想段落'}；源剧情明确的Q版、梦境、法术与特效照常表现；主观形变遵守正文声明的主观边界，现实状态及角色知情范围遵循原文。\n${displayHeader(u.unitDirection)}`
   :`${style}${active?'；已选幻想只按本单元画面正文呈现。':'；本单元不加入额外幻想段落，源剧情的法术与特效照常表现。'}`;
  const opening=u.segments?.[0]||u;
  const subjects=validVisualSubjects(u.visualSubjects)?u.visualSubjects:[];
  const subjective=subjects.filter(s=>s.domain==='subjective').map(s=>s.name),prospective=subjects.filter(s=>s.domain==='prospective').map(s=>s.name);
  const lines=[`# ${u.title}｜${num(total)}s`,`[通用提示词] ${common}`,`[场景] ${u.scene}`,`[本单元道具] ${u.props?.join('、')||'无'}`,`[本单元出场人物] ${u.participants?.join('、')||'无'}`,...(subjective.length?[`[主观视觉主体] ${subjective.join('、')}`]:[]),...(prospective.length?[`[未来图示主体] ${prospective.join('、')}`]:[]),`[本单元光影] ${displayHeader(opening.lighting)}`,`[人物状态与站位前提] ${displayHeader(opening.initialState)}`];
  for(const [i,s] of (u.shots||[]).entries()){
   const segment=u.segments?.find(segment=>segment.shotIds?.[0]===s.id);
   if(segment&&i>0){
    const last=u.shots.findIndex(shot=>shot.id===segment.shotIds.at(-1))+1;
    const previous=u.segments[u.segments.indexOf(segment)-1];
    if(segment.lighting!==previous?.lighting)lines.push(`[光影承接｜分镜${i+1}—${last}] ${segment.lighting}`);
    lines.push(`[人物状态｜分镜${i+1}起] ${segment.initialState}`);
   }
   const start=t;t+=Number.isFinite(s.duration)?s.duration:0;
   const dialogue=(s.dialogueParts||[]).map(p=>{
    const q=quotes.get(p.lineId);
    const spoken=`${quoteEdges?.first.has(p)?'':'“'}${p.text}${quoteEdges?.last.has(p)?'':'”'}`;
    if(execution){
     const channel=['VO','OS','系统音'].includes(q?.channel)?q.channel:null;
     const kind=q?.kind==='roar'?'兽吼':q?.kind==='scream'?'尖叫':q?.kind==='vocalization'?q.cue:null;
     const labels=[...new Set([channel,kind,p.presentation==='画内'?null:p.presentation].filter(has))];
     if(unitTimeline){
      let delivery=typeof p.delivery==='string'?p.delivery:'';
      if(has(q?.speaker)&&[`${q.speaker}：`,`${q.speaker}:`].some(prefix=>delivery.startsWith(prefix)))delivery=delivery.slice(q.speaker.length+1);
      delivery=delivery.trim().replace(/。$/,'');
      return `${num(start+p.voiceStart)}–${num(start+p.voiceEnd)}秒｜${q?.speaker||'未知说者'}（${[...labels,delivery].filter(has).join('·')}）：${spoken}`;
     }
     return `${q?.speaker||'未知说者'}（${[...labels,p.delivery].join('·')}｜本镜${num(p.voiceStart)}-${num(p.voiceEnd)}s）：${spoken}`;
    }
    const kind=q?.kind==='roar'?'声事件／保持兽吼声型':q?.kind==='scream'?'声事件／保持尖叫声型':null;
    return `${q?.speaker||'未知说者'}${kind?`（${kind}）`:''}，${p.presentation}，本镜${num(p.voiceStart)}-${num(p.voiceEnd)}s，演法：${p.delivery}：${spoken}`;
   }).join(unitTimeline?'\n台词：':'；')||'无';
   lines.push(`分镜${i+1}｜${num(start)}-${num(t)}s｜${num(s.duration)}s`,`机位/运镜：${s.camera}`,`画面：${s.picture}`,`光影：${s.lighting}`,`声音：${s.sound}`,`台词：${dialogue}`,`转场：${s.transition}`);
  }
  sections.push(lines.join('\n'));
 }
 return sections.join('\n\n---\n\n')+'\n';
}
