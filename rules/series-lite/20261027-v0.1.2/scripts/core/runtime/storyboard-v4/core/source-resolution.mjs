import {digest,fail} from './shared.mjs';
import {createStore} from './store.mjs';

export const SOURCE_RESOLUTION_VERSION='production-source-resolution-v1';
const text=x=>typeof x==='string'&&Boolean(x.trim())&&!x.includes('\0');
const exact=(x,keys)=>x&&typeof x==='object'&&!Array.isArray(x)&&Object.keys(x).every(k=>keys.includes(k));
const basis=source=>digest({sourceRows:source.sourceRows,dialogue:source.dialogue});
const payload=r=>({version:r.version,sourceHash:r.sourceHash,kind:r.kind,sourceRefs:r.sourceRefs,instruction:r.instruction,conflicts:r.conflicts,...(r.speaker?{speaker:r.speaker}:{}),approval:r.approval});

// Human decisions are an independent trusted input, never a field accepted from model output.
export function recordSourceResolution(d,value,approval){
 const bad=message=>fail('SOURCE_RESOLUTION',message);
 if(d.web?.sourceStale||d.operations.some(o=>o.status==='running'||o.status==='unknown'&&o.cancelAcknowledgement?.acceptUnknownUsage!==true))bad('先核对原文及未结请求，不能在派单中修改制作取舍。');
 if(d.adopted?.length||d.candidates.some(c=>c.stage!=='plan'&&(c.raw?.trim()||c.output!=null)))bad('已有付费正文须先定位局部返修，不能直接改其制作解释。');
 if(!exact(approval,['actorId','reference','answer'])||!['actorId','reference','answer'].every(k=>text(approval[k]))||approval.reference.length>500||approval.answer.length>2000)bad('缺少独立的人类明确决定及可追溯引用。');
 if(!exact(value,['sourceHash','kind','sourceRefs','instruction','conflicts','speaker'])||value.sourceHash!==basis(d.source)
  ||!['TIME_LIGHTING_SPLIT','DELAYED_BROADCAST','SPEAKER_CORRECTION'].includes(value.kind)||!text(value.instruction)||value.instruction.length>1800
  ||!Array.isArray(value.sourceRefs)||!value.sourceRefs.length||new Set(value.sourceRefs).size!==value.sourceRefs.length||!Array.isArray(value.conflicts))bad('制作解释须绑定当前完整原文、明确类型与有限坐标。');
 const refs=new Set(d.source.sourceRows.map(r=>r.id));
 if(value.sourceRefs.some(id=>!refs.has(id)))bad('不能把前情、路径或未知坐标当作本集修改范围。');
 if(value.kind==='SPEAKER_CORRECTION'){
  const q=d.source.dialogue.find(q=>q.id===value.speaker?.lineId);
  if(!exact(value.speaker,['lineId','from','to'])||!q||q.speaker!==value.speaker.from||!text(value.speaker.to)||value.speaker.to.length>60||value.speaker.to===q.speaker||!value.sourceRefs.includes(q.rowId))bad('说话人修正须精确绑定原对白，保留原句、声道和表演提示。');
 }else if(value.speaker!==undefined)bad('时段与媒介读法不能夹带改说话人。');
 const hard=new Set([...(d.production?.boundary?.agreements||[]),...(d.production?.outgoingAgreements||[]),...(d.production?.boundary?.actual?.states||[])].map(s=>s.key));
 const seen=new Set();
 for(const c of value.conflicts){
  const candidate=d.candidates.find(x=>x.id===c.candidateId),feedback=candidate?.productionFeedback;
  const conflict=feedback?.conflicts?.find(x=>digest(x)===c.conflictHash);
  if(!exact(c,['candidateId','conflictHash','needHashes'])||candidate?.stage!=='plan'||candidate.status!=='adopted'||!conflict||hard.has(conflict.key)
   ||conflict.sourceRefs.some(id=>!value.sourceRefs.includes(id))||!Array.isArray(c.needHashes)||new Set(c.needHashes).size!==c.needHashes.length||seen.has(`${c.candidateId}:${c.conflictHash}`))bad('只能解决明确批准的本集源内冲突，不能覆盖实际尾态或锁定约定。');
  seen.add(`${c.candidateId}:${c.conflictHash}`);
  for(const hash of c.needHashes){const need=feedback.needs.find(n=>digest(n)===hash);
   if(!need||![1,3].includes(need.stage)||need.sourceRefs.some(id=>!value.sourceRefs.includes(id))||!need.sourceRefs.some(id=>conflict.sourceRefs.includes(id)))bad('缺料核销必须精确绑定同一原冲突与完整本集坐标。');
  }
 }
 const result={version:SOURCE_RESOLUTION_VERSION,...structuredClone(value),approval:structuredClone(approval)};
 result.id=digest(payload(result));
 return {...result,at:new Date().toISOString()};
}

export function currentSourceResolutions(source,resolutions=[]){
 for(const r of resolutions)if(r.version!==SOURCE_RESOLUTION_VERSION||r.id!==digest(payload(r))||r.sourceHash!==basis(source))fail('SOURCE_RESOLUTION_STALE','已确认制作解释与当前原文或批准记录不匹配，不能静默忽略。');
 return resolutions;
}
export function sourceResolutionFeedback(d,candidate){
 const resolved=new Set(currentSourceResolutions(d.source,d.production?.sourceResolutions).flatMap(r=>r.conflicts.filter(c=>c.candidateId===candidate.id).map(c=>c.conflictHash)));
 if(!resolved.size)return candidate.productionFeedback;
 return {...candidate.productionFeedback,conflicts:candidate.productionFeedback.conflicts.filter(c=>!resolved.has(digest(c)))};
}
export function sourceResolutionForNeed(d,candidate,need){
 return currentSourceResolutions(d.source,d.production?.sourceResolutions).find(r=>r.conflicts.some(c=>c.candidateId===candidate.id&&c.needHashes.includes(digest(need))));
}
export function resolvedDialogue(source,resolutions=[]){
 const overrides=currentSourceResolutions(source,resolutions).filter(r=>r.kind==='SPEAKER_CORRECTION');
 return (source.dialogue||[]).map(q=>{const r=overrides.findLast(r=>r.speaker.lineId===q.id);return r?{...q,speaker:r.speaker.to}:q;});
}

// Only pre-authoring human decisions. Registration is immutable; no model or source write.
export function createSourceResolutionRegistry(directory){
 const store=createStore(directory);
 return {
  async register(binding,source,value,approval){
   if(!text(binding?.projectId)||!text(binding.episodeId)||value.conflicts?.length)fail('SOURCE_RESOLUTION','前置登记不核销任何旧任务冲突。');
   const resolution=recordSourceResolution({source,operations:[],candidates:[],adopted:[]},value,approval);
   await store.create(binding.episodeId,{projectId:binding.projectId,episodeId:binding.episodeId,sourceResolutions:[resolution]});return resolution;
  },
  async read(binding,source){
   let record;try{record=await store.read(binding.episodeId);}catch(e){if(e.code==='ENOENT')return [];throw e;}
   if(record.projectId!==binding.projectId||record.episodeId!==binding.episodeId)fail('SOURCE_RESOLUTION_STALE','前置决定不属于当前项目与本集。');
   return currentSourceResolutions(source,record.sourceResolutions);
  }
 };
}
