// Explicit interpretation of a locked VO line, never a source reclassification.
import {createHash} from 'node:crypto';
import {isDeepStrictEqual} from 'node:util';

export const SOUND_EVENT_PROFILE='source-bound-sound-events-v1';
export const SOUND_EVENT_PRESENTATION='声音事件';
const object=value=>value!==null&&typeof value==='object'&&!Array.isArray(value);
const text=value=>typeof value==='string'&&value.trim().length>0&&!/[\0\r]/.test(value);
const need=(ok,code,message)=>{if(!ok)throw Object.assign(new Error(message),{code});};
const stable=value=>Array.isArray(value)?'['+value.map(stable).join(',')+']':object(value)?'{'+Object.keys(value).sort().map(k=>JSON.stringify(k)+':'+stable(value[k])).join(',')+'}':JSON.stringify(value);
const digest=value=>createHash('sha256').update(value).digest('hex');
const fields=(value,allowed)=>object(value)&&Object.keys(value).every(k=>allowed.includes(k));
const list=value=>Array.isArray(value)?value:[];
const boundary=(value,n)=>!(n>0&&n<value.length&&value.charCodeAt(n-1)>=0xD800&&value.charCodeAt(n-1)<=0xDBFF&&value.charCodeAt(n)>=0xDC00&&value.charCodeAt(n)<=0xDFFF);

export function soundEventSourceBinding(source,quote){
 const sourceRefs=[...(quote.sourceIds??[quote.rowId])];
 const rows=sourceRefs.map(id=>{
  const row=source.sourceRows.find(row=>row.id===id);
  need(row,'SOUND_EVENT_SOURCE',`${quote.id} 声音事件的锁定源行不存在`);
  return {id:row.id,text:row.text};
 });
 const original={key:source.key,lineId:quote.id,sourceRefs,rows,speaker:quote.speaker,channel:quote.channel,
  kind:quote.kind,quote:quote.quote,cue:quote.cue??'',performanceCues:quote.performanceCues??[]};
 return {sourceRefs,quoteSha256:digest(quote.quote),sourceBindingSha256:digest(stable(original))};
}

export function soundEventDecisionMap(draft,source){
 const policy=draft?.soundEventDecisions;
 if(policy===undefined)return new Map();
 need(fields(policy,['version','items'])&&policy.version===SOUND_EVENT_PROFILE&&Array.isArray(policy.items),
  'SOUND_EVENT_PROFILE','声音事件须使用明确的source-bound-sound-events-v1逐行决定');
 const quotes=new Map((source?.dialogue??[]).map(q=>[q.id,q])),decisions=new Map();
 for(const item of policy.items){
  need(fields(item,['lineId','sourceRefs','quoteSha256','sourceBindingSha256','interpretation','reason'])&&text(item.lineId)&&!decisions.has(item.lineId),
   'SOUND_EVENT_DECISION','声音事件决定字段无效、未知或原句重复');
  const q=quotes.get(item.lineId);
  need(q,'SOUND_EVENT_SOURCE',`声音事件原句${item.lineId}不存在`);
  need(q.channel==='VO'&&q.kind==='speech','SOUND_EVENT_PROTECTED_VOICE',`${q.id} 仅源VO口语类描述可经逐句核源解释为非人声；对白、OS、系统音及既有人声声型保持`);
  need(item.interpretation==='non_speech_sound'&&text(item.reason),'SOUND_EVENT_DECISION',`${q.id} 缺少明确非人声解释或实际核源理由`);
  const binding=soundEventSourceBinding(source,q);
  need(isDeepStrictEqual(item.sourceRefs,binding.sourceRefs)&&item.quoteSha256===binding.quoteSha256&&item.sourceBindingSha256===binding.sourceBindingSha256,
   'SOUND_EVENT_SOURCE_DRIFT',`${q.id} 声音事件依据与锁定原句、说者、声道或源行不一致`);
  decisions.set(q.id,item);
 }
 return decisions;
}

export function validateSoundEvents(draft,source){
 const decisions=soundEventDecisionMap(draft,source),quotes=new Map((source?.dialogue??[]).map(q=>[q.id,q]));
 const cursors=new Map(),used=new Set();
 for(const unit of list(draft?.units))for(const shot of list(unit?.shots))for(const part of list(shot?.dialogueParts)){
  const event=part?.presentation===SOUND_EVENT_PRESENTATION,decision=decisions.get(part?.lineId);
  need(!event||decision,'SOUND_EVENT_AUTHORIZATION',`${shot.id}/${part?.lineId} 声音事件缺少同版逐行决定`);
  need(!decision||event,'SOUND_EVENT_INTERPRETATION',`${shot.id}/${part?.lineId} 同一原句不得同时解释为人声与声音事件`);
  if(!event){need(part?.sourceQuoteRange===undefined,'SOUND_EVENT_RANGE',`${shot.id} 普通人声不接受声音事件专用来源区间`);continue;}
  const q=quotes.get(part.lineId),range=part.sourceQuoteRange,start=cursors.get(q.id)??0;
  need(Array.isArray(range)&&range.length===2&&range.every(Number.isSafeInteger)&&range[0]===start&&range[1]>range[0]&&range[1]<=q.quote.length,
   'SOUND_EVENT_RANGE',`${shot.id}/${q.id} 声音事件原句区间无效、重叠或跳字`);
  need(boundary(q.quote,range[0])&&boundary(q.quote,range[1]),'SOUND_EVENT_RANGE',`${shot.id}/${q.id} 声音事件区间不能切开Unicode代理对`);
  need(part.text===q.quote.slice(range[0],range[1]),'SOUND_EVENT_TEXT',`${shot.id}/${q.id} 声音事件追溯文本被修改`);
  need(!['speaker','channel','kind','cue','performanceCues'].some(k=>Object.hasOwn(part,k)),
   'SOUND_EVENT_SOURCE_FIELDS',`${shot.id}/${q.id} 说者、声道及原声元信息只能取锁定源`);
  need(text(part.delivery),'SOUND_EVENT_DELIVERY',`${shot.id}/${q.id} 缺少实际声音演绎`);
  need(Number.isFinite(part.voiceStart)&&Number.isFinite(part.voiceEnd)&&Number.isFinite(shot.duration)&&part.voiceStart>=0&&part.voiceEnd>part.voiceStart&&part.voiceEnd<=shot.duration,
   'SOUND_EVENT_WINDOW',`${shot.id}/${q.id} 声音事件时间窗必须在本镜实际时长内`);
  need(q.sourceIds.every(ref=>(shot.sourceRefs??[]).includes(ref)),'SOUND_EVENT_SOURCE',`${shot.id}/${q.id} 声音事件镜头须引用本句全部源行`);
  cursors.set(q.id,range[1]);used.add(q.id);
 }
 for(const id of decisions.keys())need(used.has(id)&&cursors.get(id)===quotes.get(id).quote.length,'SOUND_EVENT_COVERAGE',`${id} 已决定的声音事件须完整逐字覆盖一次`);
 return decisions;
}

export function soundEventDelivery(part,from,to){
 return `${from}–${to}秒｜非人声：${part.delivery}`;
}
