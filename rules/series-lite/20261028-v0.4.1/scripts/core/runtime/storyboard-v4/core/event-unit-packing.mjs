import {fail} from './shared.mjs';
import {resolveSourceRef} from './source.mjs';
import {validVisualSubjects,mergeVisualSubjects} from './visual-subjects.mjs';

export const UNIT_PACKING_POLICY='event-first-v1';
const has=x=>typeof x==='string'&&!!x.trim();
const unique=xs=>[...new Set(xs)];
const required=r=>has(r.text)&&!['metadata','blank','punctuation'].includes(r.kind)&&!/^\s*[！!？?。，,.、…；;：:]+\s*$/.test(r.text);
const emotionOK=e=>e&&['subject','contextTone','audienceEffect','change','landing'].every(k=>has(e[k]))&&Array.isArray(e.retain)&&e.retain.length&&e.retain.every(has);
const combine=(items,keys)=>Object.fromEntries([...keys.map(k=>[k,unique(items.map(x=>x[k])).join('；')]),['retain',unique(items.flatMap(x=>x.retain))]]);
const boundaryOK=(b,kinds)=>b&&kinds.includes(b.kind)&&has(b.reason);

// The planner supplies semantic event boundaries; code only enforces that explicit ownership.
export function normalizeEventPlan(plan,source){
 if(plan.unitPackingPolicy===undefined)return;
 if(plan.unitPackingPolicy!==UNIT_PACKING_POLICY)fail('PLAN_EVENT_POLICY','未知事件单元策略');
 if(!Array.isArray(plan.events)||!plan.events.length)fail('PLAN_EVENTS','完整事件与边界依据缺失');
 const rowOrder=new Map(source.sourceRows.map((r,i)=>[r.id,i])),production=source.sourceRows.filter(required).map(r=>r.id),eventOwners=new Map(),eventIds=new Set();
 let previousEnd=-1;
 for(const [i,event] of plan.events.entries()){
  if(!has(event?.id)||['__proto__','constructor','prototype'].includes(event.id)||eventIds.has(event.id)||!has(event.sceneKey)||!emotionOK(event.emotionContract)||!boundaryOK(event.boundaryBefore,['episode_start','continuous','new_event','time_space_jump']))fail('PLAN_EVENT','事件标识、场景、完整情绪契约或边界依据无效');
  eventIds.add(event.id);
  if((i===0)!==(event.boundaryBefore.kind==='episode_start'))fail('PLAN_EVENT_BOUNDARY','仅首个事件使用episode_start；后续须说明连续、独立事件或时空跳转');
  if(!Array.isArray(event.sourceRefs))fail('PLAN_EVENT_REFS','事件须给稳定源引用');
  event.sourceRefs=unique(event.sourceRefs.map(r=>resolveSourceRef(source,r).id));
  for(const q of source.dialogue)if(q.sourceIds.some(r=>event.sourceRefs.includes(r)))event.sourceRefs=unique([...event.sourceRefs,...q.sourceIds]);
  event.sourceRefs.sort((a,b)=>rowOrder.get(a)-rowOrder.get(b));
  const covered=event.sourceRefs.filter(r=>production.includes(r)),indices=covered.map(r=>production.indexOf(r));
  if(!indices.length||indices[0]<=previousEnd||indices.at(-1)-indices[0]+1!==indices.length)fail('PLAN_EVENT_CONTINUITY','完整事件须按源顺序连续、非空且不交错');
  previousEnd=indices.at(-1);
  for(const ref of covered){if(eventOwners.has(ref))fail('PLAN_EVENT_OVERLAP','一个制作源只能归一个完整事件');eventOwners.set(ref,event.id);}
  if(i&&event.boundaryBefore.kind==='continuous'&&event.sceneKey!==plan.events[i-1].sceneKey)fail('PLAN_EVENT_BOUNDARY','跨场景不可声明为同一连续事件');
 }
 if(production.some(r=>!eventOwners.has(r)))fail('PLAN_EVENT_COVERAGE','实际剧情缺少完整事件归属');
 const originalEvents=new Map(plan.events.map(e=>[e.id,e])),aliases=new Map(),groups=[];
 for(const event of plan.events){
  const previous=groups.at(-1);
  if(previous&&event.boundaryBefore.kind==='continuous'){
   previous.eventSegments??=[structuredClone(previous)];previous.eventSegments.push(...(event.eventSegments||[structuredClone(event)]));
   previous.sourceRefs=unique([...previous.sourceRefs,...event.sourceRefs]);
   previous.emotionContract=combine(previous.eventSegments.map(e=>e.emotionContract),['subject','contextTone','audienceEffect','change','landing']);
   aliases.set(event.id,previous.id);
  }else{groups.push(structuredClone(event));aliases.set(event.id,event.id);}
 }
 const grouped=new Map();
 for(const w of plan.workPackages){
  const event=originalEvents.get(w.eventId);
  if(!event||w.sourceRefs.some(r=>!event.sourceRefs.includes(r)))fail('PLAN_EVENT_OWNER',`工作包${w.id}须唯一归属所引完整事件`);
  const key=aliases.get(w.eventId);if(!grouped.has(key))grouped.set(key,[]);grouped.get(key).push(w);
 }
 const owners=new Map(),packages=[];
 for(const event of groups){
  const list=grouped.get(event.id)||[];if(!list.length)fail('PLAN_EVENT_OWNER',`事件${event.id}缺少主创工作包`);
  list.sort((a,b)=>Math.min(...a.sourceRefs.map(r=>rowOrder.get(r)))-Math.min(...b.sourceRefs.map(r=>rowOrder.get(r))));
  const first=list[0],w={...first,eventId:event.id,sourceRefs:unique(list.flatMap(x=>x.sourceRefs)).sort((a,b)=>rowOrder.get(a)-rowOrder.get(b)),emotionContract:structuredClone(event.emotionContract),beatContracts:list.flatMap(x=>x.beatContracts||[{packageId:x.id,sourceRefs:[...x.sourceRefs],emotionContract:structuredClone(x.emotionContract)}])};
  for(const key of ['entryFactIds','contextRefs','dependsOn','scenarioKeys'])w[key]=unique(list.flatMap(x=>x[key]||[]));
  for(const old of list)owners.set(old.id,w.id);packages.push(w);
 }
 for(const w of packages)w.dependsOn=unique(w.dependsOn.map(id=>owners.get(id)||id)).filter(id=>id!==w.id);
 for(const u of plan.provisionalUnits)u.packageId=owners.get(u.packageId)||u.packageId;
 plan.events=groups;plan.workPackages=packages;
 plan.eventPacking=packages.map(w=>{const from=unique(w.beatContracts.map(b=>b.packageId));return {eventId:w.eventId,ownerId:w.id,from,action:from.length>1?'merge':'retain',reason:from.length>1?'complete_event_one_owner':'single_event_owner'};});
}

const seconds=u=>Array.isArray(u?.shots)&&u.shots.length&&u.shots.every(s=>Number.isFinite(s?.duration)&&s.duration>0)?u.shots.reduce((n,s)=>n+s.duration,0):NaN;
const segments=u=>u.segments||[Object.fromEntries([...Object.entries(u).filter(([k])=>k!=='shots'&&k!=='segments'),['shotIds',u.shots.map(s=>s.id)]])];
const reasons={different_event:'不同完整事件，保留边界',unknown_event:'事件归属未确认，保留边界',explicit_boundary:'明确独立事件或时空跳转，保留边界',unconfirmed_boundary:'缺少明确连续依据，保留边界',scene_changed_or_unconfirmed:'场景头不一致，未猜测为同场',capacity_exceeded:'自然镜长合计超过所选容量，不压秒、不删表演',invalid_unit:'现稿字段或时长未完整，交既有校验处理',event_source_mismatch:'镜头源引用与事件归属不符，未合并'};

export function packEventUnits(draft,{plan,settings,unitPackingPolicy,stage='finish'}={}){
 const changes=[];
 if(unitPackingPolicy===undefined)return changes;
 if(unitPackingPolicy!==UNIT_PACKING_POLICY||plan?.unitPackingPolicy!==UNIT_PACKING_POLICY||!Array.isArray(plan.events)||!plan.events.length)fail('DRAFT_EVENT_POLICY','新版整编必须携带明确完整事件计划');
 if(!Array.isArray(draft?.units)||!draft.units.length||draft.units.some(u=>!has(u?.eventId)||!plan.events.some(e=>e.id===u.eventId)||!boundaryOK(u.boundaryBefore,['episode_start','continuous','capacity','new_event','time_space_jump'])))fail('DRAFT_EVENT_METADATA','新版单元必须有有效事件归属和明确边界，不能静默保留碎分');
 if(stage!=='finish'||draft.units.length<2)return changes;
 const cap=settings?.maxUnitSeconds??30;if(![15,30].includes(cap))return changes;
 const events=new Map(plan.events.map(e=>[e.id,e])),packed=[];
 for(const next of draft.units){
  const prior=packed.at(-1);if(!prior){packed.push(next);continue;}
  const event=events.get(prior.eventId);let reason;
  if(!event||!events.has(next?.eventId))reason='unknown_event';
  else if(prior.eventId!==next.eventId)reason='different_event';
  else if(['new_event','time_space_jump','episode_start'].includes(next.boundaryBefore?.kind))reason='explicit_boundary';
  else if(!boundaryOK(next.boundaryBefore,['continuous','capacity']))reason='unconfirmed_boundary';
  else if(!has(prior.scene)||prior.scene!==next.scene)reason='scene_changed_or_unconfirmed';
  else if(!Number.isFinite(seconds(prior))||!Number.isFinite(seconds(next))||[prior,next].some(u=>!u.intent||!Array.isArray(u.intent.retain)||!['props','participants'].every(k=>Array.isArray(u[k]))||!['lighting','initialState','exitState'].every(k=>has(u[k]))||Object.hasOwn(u,'visualSubjects')&&!validVisualSubjects(u.visualSubjects)))reason='invalid_unit';
  else if([prior,next].some(u=>u.shots.some(s=>!Array.isArray(s.sourceRefs)||s.sourceRefs.some(r=>!event.sourceRefs.includes(r)))))reason='event_source_mismatch';
  else if(seconds(prior)+seconds(next)>cap+1e-6)reason='capacity_exceeded';
  if(reason){changes.push({coordinate:next?.id||'unit',field:'unitBoundary',action:'retain',reason,detail:reasons[reason]});packed.push(next);continue;}
  const from=[prior.id,next.id],parts=[...segments(prior),...segments(next)];
  const edge=prior.shots.at(-1),transition=edge.transition;
  if(has(transition)&&/(?:下一单元|下个单元)/.test(transition)&&!/(?:外部|另场|次日|后续事件)/.test(transition)&&!draft.units.some(u=>u?.id!==next.id&&has(u?.id)&&transition.includes(u.id))){
   edge.transition=transition.replace(/下一单元|下个单元/g,'下一段');
   changes.push({coordinate:edge.id,field:'transition',action:'normalize',from:transition,to:edge.transition,reason:'merged_internal_boundary'});
  }
  const lighting=unique(parts.map(s=>s.lighting)).length===1?parts[0].lighting:parts.map(s=>`【${s.shotIds[0]}—${s.shotIds.at(-1)}】${s.lighting}`).join('\n');
  const initialState=[parts[0].initialState,...parts.slice(1).map(s=>`【${s.shotIds[0]}起承接】${s.initialState}`)].join('\n');
  const directed=parts.filter(s=>has(s.unitDirection));
  const unitDirection=directed.length===parts.length&&unique(directed.map(s=>s.unitDirection)).length===1?directed[0].unitDirection:directed.map(s=>`【${s.shotIds[0]}—${s.shotIds.at(-1)}】${s.unitDirection}`).join('\n');
  packed[packed.length-1]={...prior,title:unique(parts.map(s=>s.title)).join(' / '),...(directed.length?{unitDirection}:{}),intent:combine(parts.map(s=>s.intent),['subject','change','landing']),props:unique(parts.flatMap(s=>s.props)),participants:unique(parts.flatMap(s=>s.participants)),...([prior,next].some(u=>Object.hasOwn(u,'visualSubjects'))?{visualSubjects:mergeVisualSubjects([prior,next])}:{}),lighting,initialState,exitState:next.exitState,shots:[...prior.shots,...next.shots],segments:parts};
  for(const f of Array.isArray(draft.fantasyRealization)?draft.fantasyRealization:[])if(Array.isArray(f?.unitIds))f.unitIds=unique(f.unitIds.map(id=>id===next.id?prior.id:id));
  changes.push({coordinate:prior.id,field:'units',action:'merge',from,to:prior.id,eventId:prior.eventId,seconds:Number((seconds(prior)+seconds(next)).toFixed(3)),reason:'continuous_event_within_capacity',detail:'同一连续事件在所选容量内合并；画面、声画、镜长及台词未改，仅必要的内部单元转场用语另记归一；原分段头保存在segments'});
 }
 draft.units=packed;return changes;
}
