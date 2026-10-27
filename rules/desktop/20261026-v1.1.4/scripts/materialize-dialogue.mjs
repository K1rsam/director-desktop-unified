#!/usr/bin/env node
// Select exact normalized-quote UTF-16 ranges; never interpret prose as code.
import {readFile,writeFile} from 'node:fs/promises';
import {realpathSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {isDeepStrictEqual} from 'node:util';
import {resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {normalizeSource} from './core/runtime/storyboard-v4/core/source.mjs';
import {SOUND_EVENT_PRESENTATION,soundEventSourceBinding,validateSoundEvents} from './core/runtime/storyboard-v4/core/sound-events.mjs';

const PROFILE='normalized-quote-utf16-v1';
const hash=raw=>createHash('sha256').update(raw).digest('hex');
const need=(ok,code,message)=>{if(!ok)throw Object.assign(new Error(message),{code});};
const object=v=>v!==null&&typeof v==='object'&&!Array.isArray(v);
const text=v=>typeof v==='string'&&v.length>0;
const stable=v=>{
 if(Array.isArray(v))return '['+v.map(stable).join(',')+']';
 if(object(v))return '{'+Object.keys(v).sort().map(k=>JSON.stringify(k)+':'+stable(v[k])).join(',')+'}';
 return JSON.stringify(v);
};
function sourceOf(raw){
 const decoded=new TextDecoder('utf-8',{fatal:true}).decode(raw);
 const source=normalizeSource(JSON.parse(decoded));
 need(!source.diagnostics?.length,'SOURCE_DIAGNOSTICS','源仍有诊断，先处理接入');
 need(Array.isArray(source.dialogue),'SOURCE_DIALOGUE','缺少规范化原声目录');
 const seen=new Set();
 for(const q of source.dialogue){
  need(text(q.id)&&typeof q.quote==='string'&&!seen.has(q.id),'SOURCE_DIALOGUE','源原句ID无效或重复');
  seen.add(q.id);
 }
 return source;
}
const boundary=(s,n)=>!(n>0&&n<s.length&&s.charCodeAt(n-1)>=0xD800&&s.charCodeAt(n-1)<=0xDBFF&&s.charCodeAt(n)>=0xDC00&&s.charCodeAt(n)<=0xDFFF);

export function inspectDialogue(sourceBytes){
 const source=sourceOf(sourceBytes);
 return {coordinateProfile:PROFILE,sourceSha256:hash(sourceBytes),dialogue:source.dialogue.map(q=>({
  lineId:q.id,quote:q.quote,utf16Length:q.quote.length,speaker:q.speaker,channel:q.channel,
  kind:q.kind,cue:q.cue??'',performanceCues:q.performanceCues??[],...soundEventSourceBinding(source,q)
 }))};
}

export function materializeDialogue(sourceBytes,selection){
 const source=sourceOf(sourceBytes),sourceSha256=hash(sourceBytes);
 need(object(selection)&&selection.version==='desktop-dialogue-selection-v1','SELECTION_VERSION','未知原声选择协议');
 need(selection.coordinateProfile===PROFILE,'COORDINATE_PROFILE','须使用规范化quote的相对UTF16坐标');
 need(selection.sourceSha256===sourceSha256,'SOURCE_DRIFT','source文件hash不匹配，不能沿用旧区间');
 const allIds=source.dialogue.map(q=>q.id),scope=selection.scopeLineIds??allIds;
 need(Array.isArray(scope)&&scope.every(text)&&new Set(scope).size===scope.length,'SCOPE','责任原句必须唯一');
 need(isDeepStrictEqual(scope,allIds.filter(id=>scope.includes(id))),'SCOPE','责任原句未知或未按源序');
 const byId=new Map(source.dialogue.map(q=>[q.id,q])),order=new Map(scope.map((id,i)=>[id,i]));
 const draft=structuredClone(selection.draft),cursor=new Map(scope.map(id=>[id,0])),ranges=[];
 need(object(draft)&&Array.isArray(draft.units)&&draft.units.length,'DRAFT','缺少完整单元数组');
 let last=-1;const unitIds=new Set(),shotIds=new Set();
 for(const u of draft.units){
  need(object(u)&&text(u.id)&&!unitIds.has(u.id)&&Array.isArray(u.shots)&&u.shots.length,'UNITS','单元ID重复或缺镜头');
  unitIds.add(u.id);
  for(const s of u.shots){
   need(object(s)&&text(s.id)&&!shotIds.has(s.id)&&Array.isArray(s.dialogueParts),'SHOTS','镜头ID重复或缺dialogueParts');
   shotIds.add(s.id);
   s.dialogueParts=s.dialogueParts.map((p,index)=>{
    need(object(p)&&order.has(p.lineId),'LINE_ID',`${u.id}/${s.id} 原句不在责任范围`);
    need(!['text','speaker','channel','kind','cue','performanceCues','speakers','sourceQuoteRange'].some(k=>Object.hasOwn(p,k)),'MODEL_SOURCE_FIELDS','作者不能重抄或覆盖源原话/说者/声道/提示/派生区间');
    const q=byId.get(p.lineId),start=p.quoteStart,end=p.quoteEnd,i=order.get(p.lineId);
    need(Number.isSafeInteger(start)&&Number.isSafeInteger(end)&&start>=0&&end>start&&end<=q.quote.length,'RANGE',`${s.id}/${p.lineId} 原句区间无效`);
    need(boundary(q.quote,start)&&boundary(q.quote,end),'UTF16_BOUNDARY',`${s.id}/${p.lineId} 区间切开Unicode代理对`);
    need(i>=last,'DIALOGUE_ORDER',`${s.id}/${p.lineId} 原句倒序`);
    for(let n=0;n<i;n++)need(cursor.get(scope[n])===byId.get(scope[n]).quote.length,'DIALOGUE_GAP','前句尚未完整覆盖');
    need(start===cursor.get(p.lineId),'DIALOGUE_GAP',`${s.id}/${p.lineId} 跳字、重复或区间不连续`);
    last=i;cursor.set(p.lineId,end);
    const {quoteStart,quoteEnd,...rest}=p;
    ranges.push({unitId:u.id,shotId:s.id,partIndex:index,lineId:p.lineId,quoteStart:start,quoteEnd:end});
    return {...rest,text:q.quote.slice(start,end),...(p.presentation===SOUND_EVENT_PRESENTATION?{sourceQuoteRange:[start,end]}:{})};
   });
  }
 }
 for(const id of scope)need(cursor.get(id)===byId.get(id).quote.length,'DIALOGUE_COVERAGE',`${id} 原声未完整覆盖`);
 validateSoundEvents(draft,source);
 const receipt={coordinateProfile:PROFILE,sourceSha256,scopeLineIds:[...scope],ranges,draftSha256:hash(stable(draft)),
  semanticReview:'NOT_RUN',timingMeasured:false};
 return {version:'desktop-materialized-draft-v1',draft,receipt};
}

export function verifyMaterialized(sourceBytes,result){
 need(object(result)&&result.version==='desktop-materialized-draft-v1'&&object(result.receipt),'RESULT_VERSION','未知物化稿协议');
 const {receipt}=result;
 need(receipt.coordinateProfile===PROFILE&&Array.isArray(receipt.ranges),'RECEIPT','区间回执无效');
 need(receipt.draftSha256===hash(stable(result.draft)),'DRAFT_DRIFT','物化稿hash变化');
 const selection=structuredClone(result.draft),entries=new Map();
 for(const r of receipt.ranges){
  need(object(r)&&text(r.unitId)&&text(r.shotId)&&Number.isSafeInteger(r.partIndex)&&r.partIndex>=0,'RECEIPT','无效片段坐标');
  const key=JSON.stringify([r.unitId,r.shotId,r.partIndex]);need(!entries.has(key),'RECEIPT','重复片段坐标');entries.set(key,r);
 }
 let used=0;
 for(const u of selection.units??[])for(const s of u.shots??[])s.dialogueParts=(s.dialogueParts??[]).map((p,i)=>{
  const r=entries.get(JSON.stringify([u.id,s.id,i]));
  need(r&&r.lineId===p.lineId,'RECEIPT','回执与实际原声片段不对应');used++;
  const {text:spoken,sourceQuoteRange,...rest}=p;
  return {...rest,quoteStart:r.quoteStart,quoteEnd:r.quoteEnd};
 });
 need(used===receipt.ranges.length,'RECEIPT','存在无对应镜头的区间');
 const replay=materializeDialogue(sourceBytes,{version:'desktop-dialogue-selection-v1',coordinateProfile:PROFILE,
  sourceSha256:receipt.sourceSha256,scopeLineIds:receipt.scopeLineIds,draft:selection});
 need(isDeepStrictEqual(replay,result),'RECEIPT_DRIFT','原话或回执无法从锁定源精确重现');
 return {ok:true,sourceSha256:receipt.sourceSha256,lineCount:receipt.scopeLineIds.length,partCount:used,
  semanticReview:'NOT_RUN',timingMeasured:false};
}

async function main(){
 const [mode,...args]=process.argv.slice(2),flags={};
 need(['inspect','build','verify'].includes(mode),'USAGE','使用 inspect/build/verify --source FILE [--input FILE] [--output NEWFILE]');
 for(let i=0;i<args.length;i+=2){
  need(['--source','--input','--output'].includes(args[i])&&text(args[i+1])&&!args[i+1].startsWith('--')&&!Object.hasOwn(flags,args[i]),'USAGE','未知、重复或缺值参数');
  flags[args[i]]=args[i+1];
 }
 need(text(flags['--source']),'USAGE','缺少--source');
 const raw=await readFile(flags['--source']);
 if(mode==='inspect'){
  need(!flags['--input']&&!flags['--output'],'USAGE','inspect仅接受--source');
  console.log(JSON.stringify(inspectDialogue(raw),null,2));return;
 }
 need(text(flags['--input']),'USAGE','缺少--input');
 const input=JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(await readFile(flags['--input'])));
 if(mode==='verify'){
  need(!flags['--output'],'USAGE','verify不写输出');
  console.log(JSON.stringify(verifyMaterialized(raw,input)));return;
 }
 need(text(flags['--output']),'USAGE','缺少--output');
 const result=materializeDialogue(raw,input);
 await writeFile(flags['--output'],JSON.stringify(result,null,2)+'\n',{encoding:'utf8',flag:'wx'});
 console.log(JSON.stringify({ok:true,status:'candidate',output:resolve(flags['--output']),lineCount:result.receipt.scopeLineIds.length,
  partCount:result.receipt.ranges.length,semanticReview:'NOT_RUN',adopted:false}));
}
if(process.argv[1]&&realpathSync(process.argv[1])===realpathSync(fileURLToPath(import.meta.url))){
 main().catch(e=>{console.error(JSON.stringify({ok:false,code:e.code??'ERROR',message:e.message}));process.exitCode=1;});
}
