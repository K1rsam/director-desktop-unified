import {fail} from './shared.mjs';
import {createHash} from 'node:crypto';
import {inlinePerformanceProjection,compactPerformanceHeadProjection,sourceVoiceProjection,sourceBarePerformanceProjection,sourceHeadPrefix,sourceUtteranceProperty} from '../../inline-performance.mjs';

export const STANDARD_EPISODE_PROTOCOL='standard-episode-v1';
const voices={SPEECH:'现场对白',OS:'OS',VO:'VO',NARRATION:'VO',SYSTEM:'系统音'};
const check=(ok,code,message)=>{if(!ok)fail(code,message);};
const canonical=value=>Array.isArray(value)?value.map(canonical):value&&typeof value==='object'?Object.fromEntries(Object.entries(value).sort(([a],[b])=>a<b?-1:a>b?1:0).map(([key,item])=>[key,canonical(item)])):value;
const fingerprint=value=>createHash('sha256').update(JSON.stringify(canonical(value)),'utf16le').digest('hex');
const declaredSpeakerForm=(text,name)=>text.split(/\r\n|\r|\n/).some(line=>/^\s*(?:出场人物|人物|角色)[ \t]*[:：]/u.test(line)&&line.replace(/^\s*(?:出场人物|人物|角色)[ \t]*[:：]/u,'').split(/[、,，;；]/u).some(item=>item.trim()===name));
function compactVoice(source,child,v5=false){
 const token=(v5?compactPerformanceHeadProjection(source,child.speaker.parts,child.cue?.parts,child.channelSource?.parts):null)??sourceVoiceProjection(source,child.speaker),field=child.channelSource;
 if(!token)return null;
 check(child.channel===token.text.toUpperCase()&&field?.text===token.text&&field.parts?.[0]?.start===token.start&&field.parts.at(-1)?.end===token.end,'STANDARD_COMPACT_VOICE','贴写声道必须精确绑定原文，不得改成现场对白');
 return token;
}
function postColonFields(source,child){
 const start=child?.speaker?.parts?.[0]?.start,end=child?.textAssembly?.parts?.[0]?.start;
 if(!Number.isSafeInteger(start)||!Number.isSafeInteger(end))return false;
 const header=/^[^:：()（）\r\n]+?(?:[（(][^()（）\r\n]*[）)])?(?:(?<=[）)])[\t \u00a0\u3000]*(?:OS|VO))?[\t ]*[:：]/iu.exec(source.slice(start,end));
 return Boolean(header&&[child.cue?.parts?.[0]?.start,child.channelSource?.parts?.[0]?.start].some(at=>Number.isSafeInteger(at)&&at>=start+header[0].length));
}
function postCueHeader(source,start,bodyStart,child){
 const match=/^[\t \uFEFF]*([^:：()（）\r\n]+?)(?:（([^()（）\r\n]*)）|\(([^()（）\r\n]*)\))?(?:(?<=[）)])[\t \u00a0\u3000]*(?:OS|VO))?[\t ]*[:：][\t ]*(?:（([^()（）\r\n]*)）|\(([^()（）\r\n]*)\))[\t \r\n]*$/diu.exec(source.slice(start,bodyStart));
 check(match&&!/^[（(]/.test(source.slice(bodyStart)),'STANDARD_POST_CUE_HEADER','后置提示头或重复括号边界不明确');
 const post=match[4]??match[5],bounds=match.indices;
 check(post.trim()&&!/[:：“”‘’「」『』"'△▲]/u.test(post),'STANDARD_POST_CUE_CONTENT','后置提示不得隐藏引述、说者头或动作标记');
 const name={start:start+bounds[1][0]+match[1].length-match[1].trimStart().length,end:start+bounds[1][0]+match[1].trimEnd().length};
 const bare=sourceBarePerformanceProjection(source,child.speaker,child.cue);if(bare&&bare.end<=name.end)name.end=child.speaker.parts.at(-1).end;
 const compact=compactVoice(source,child);if(compact&&name.end===compact.end)name.end=name.start+source.slice(name.start,compact.start).trimEnd().length;
 const preSlot=match[2]===undefined?3:2,qualifiedEnd=bounds[preSlot]?start+bounds[preSlot][1]+1:name.end,qualified=(!compact||compact.start>=qualifiedEnd)&&qualifiedEnd>name.end&&child.speaker.text===source.slice(name.start,qualifiedEnd)&&!/^(?:OS|VO|旁白|画外音|内心独白|系统音)$/iu.test(match[preSlot].trim())&&declaredSpeakerForm(source,child.speaker.text);
 if(qualified)name.end=qualifiedEnd;
 const exact=(field,expected)=>expected?Boolean(field&&field.parts?.[0]?.start===expected.start&&field.parts.at(-1)?.end===expected.end&&field.text===source.slice(expected.start,expected.end)):field==null;
 check(exact(child.speaker,name)&&/^[\p{L}\p{N}·・ _.-]+(?:、[\p{L}\p{N}·・ _.-]+)*$/u.test(qualified?match[1].trim():child.speaker.text)&&!/^\d+$/.test(child.speaker.text),'STANDARD_POST_CUE_SPEAKER','说者须精确绑定原文头');
 const tokens=(qualified?[4,5]:[2,3,4,5]).flatMap(slot=>match[slot]===undefined?[]:[...match[slot].matchAll(/[^,， \t]+/g)].flatMap(token=>{
  const from=start+bounds[slot][0]+token.index,end=from+token[0].length;
  return slot>=4&&/^(?:OS|VO)(?=\p{Script=Han})/iu.test(token[0])?[{slot,text:token[0].slice(0,2),start:from,end:from+2},{slot,text:token[0].slice(2),start:from+2,end}]:[{slot,text:token[0],start:from,end}];
 }));
 if(compact)tokens.unshift({slot:1,...compact});
 if(bare)tokens.unshift({slot:1,text:child.cue.text,start:bare.start,end:bare.end});
 check(!tokens.some(token=>token.slot>=4&&!/^(OS|VO)$/i.test(token.text)&&/\b(?:OS|VO)\b|\b(?:O\.S\.?|V\.O\.?)|旁白|画外音|内心独白|系统音/i.test(token.text)),'STANDARD_POST_CUE_CHANNEL','后置声道不能降级为普通表演提示');
 const channels=tokens.filter(token=>/^(OS|VO)$/i.test(token.text)),cues=tokens.filter(token=>!channels.includes(token));
 check(channels.length<=1&&new Set(cues.map(token=>token.slot)).size<=1,'STANDARD_POST_CUE_AMBIGUOUS','前后提示或声道归属不明确');
 const cueRange=cues.length?{start:cues[0].start,end:cues.at(-1).end}:null;
 const expected=channels[0]?.text.toUpperCase()??({旁白:'NARRATION',画外音:'VO',系统音:'SYSTEM'})[child.speaker.text]??'SPEECH';
 check(exact(child.cue,cueRange)&&exact(child.channelSource,channels[0]??null)&&child.channel===expected,'STANDARD_POST_CUE_FIELDS','表演提示或声道不等于对应源字段');
}
function groupUtterance(text,speaker='',cue=''){
 const stack=[],pairs={'“':'”','‘':'’','「':'」','『':'』','"':'"',"'":"'",'【':'】','[':']'};
 for(let i=0;i<text.length;i++){
  const c=text[i];if(c==='\\'){i++;continue;}
  if((c==="'"||c==='’')&&/[A-Za-z0-9]/.test(text[i-1]??'')&&/[A-Za-z0-9]/.test(text[i+1]??''))continue;
  if(stack.at(-1)===c){stack.pop();continue;}
  if(c==="'"&&!stack.includes("'")&&/[sS]/.test(text[i-1]??'')&&/\s/.test(text[i+1]??''))continue;
  if(pairs[c]&&(!/[【\[]/u.test(c)||!stack.length||/[】\]]/u.test(stack.at(-1)))){stack.push(pairs[c]);continue;}
  check(!/[”’」』]/.test(c),'STANDARD_GROUP_QUOTE','对白分组的引号边界不确定');if(stack.length)continue;
  check(!/[△▲]/.test(c),'STANDARD_GROUP_ACTION','对白分组不能把动作标记吞入对白');
  if(/[:：]/.test(c)){
   const label=/(?:^|[。.!！？?;；,， \t\r\n])([^。.!！？?;；,，:：()（） \t\r\n]+(?:[（(][^()（）]*[）)])?)[ \t]*$/u.exec(text.slice(0,i))?.[1]?.trim();
   if(!label||sourceUtteranceProperty(speaker,label,cue)||/^(?:时间层|时间|地点|场景|天气|内外|道具)$/.test(label)||/[说问道答喊叫]$/.test(label)&&/^[“‘「『"']/.test(text.slice(i+1).trimStart()))continue;
   check(!/^[\p{L}\p{N}·・ _.-]+(?:[（(][^()（）]*[）)])?$/u.test(label),'STANDARD_GROUP_NESTED_HEAD','对白子目录不能隐藏另一说者头');
  }
 }
 check(!stack.length,'STANDARD_GROUP_QUOTE','对白分组的引号未闭合');
}

// Independent lexical check on original bytes; a receipt or child label cannot declare arbitrary text a subtitle.
function subtitleSpans(text){
 const spans=[],stack=[],pairs={'“':'”','‘':'’','「':'」','『':'』','"':'"',"'":"'",'【':'】','[':']'};
 for(let i=0;i<text.length;i++){
  const c=text[i];if(c==='\\'){i++;continue;}
  if((c==="'"||c==='’')&&/[A-Za-z0-9]/.test(text[i-1]??'')&&/[A-Za-z0-9]/.test(text[i+1]??''))continue;
  if(stack.at(-1)===c){stack.pop();continue;}
  if(c==="'"&&!stack.includes("'")&&/[sS]/.test(text[i-1]??'')&&/\s/.test(text[i+1]??''))continue;
  if(!stack.length&&/^[【\[](?:字幕|SUBTITLE)(?=[：:\t \]】]|$)/iu.test(text.slice(i))){
   const m=/^【(字幕|SUBTITLE)[：:]([^【】\r\n]+)】/iu.exec(text.slice(i));
   check(m&&m[2].trim(),'STANDARD_BODY_CAPTION','明确字幕必须闭合、非空且不嵌套或跨行');
   const bodyStart=i+1+m[1].length+1;
   spans.push({start:i,end:i+m[0].length,labelStart:i+1,labelEnd:i+1+m[1].length,bodyStart,bodyEnd:bodyStart+m[2].length});
   i+=m[0].length-1;continue;
  }
  if(pairs[c]&&(!/[【\[]/u.test(c)||!stack.length||/[】\]]/u.test(stack.at(-1))))stack.push(pairs[c]);
 }
 return spans;
}

/** V3 has no extension slots inside mixed children; unknown structure must not disappear in projection. */
function mixedChildShape(child,performance=false){
 const shape=(value,keys,optional=[])=>check(value&&typeof value==='object'&&!Array.isArray(value)&&[Object.prototype,null].includes(Object.getPrototypeOf(value))&&Object.keys(value).every(key=>[...keys,...optional].includes(key))&&keys.every(key=>Object.hasOwn(value,key)),'STANDARD_BODY_SHAPE','混合子字段含未知或缺失结构，不能静默丢弃');
 const parts=value=>{check(Array.isArray(value),'STANDARD_BODY_SHAPE','V3来源片段须为数组');for(const part of value)shape(part,['kind','unitId','start','end']);};
 const mapped=value=>{shape(value,['text','parts']);parts(value.parts);};
 const dialogue=child?.kind==='DIALOGUE';
 shape(child,dialogue?['kind','text','textAssembly','layout','speaker','channel','channelSource','cue']:['kind','text','textAssembly','layout','label'],dialogue&&performance?['performance']:[]);
 shape(child.textAssembly,['parts']);parts(child.textAssembly.parts);parts(child.layout);
 if(dialogue){mapped(child.speaker);if(child.cue!==null)mapped(child.cue);if(child.channelSource!==null)mapped(child.channelSource);}
 else mapped(child.label);
}

/** Copy a compiled source slice into local coordinates, never re-infer a speaker from a colon. */
export function standardPackageSource(pkg,{key,title,context,contextRows=[],sourceText}){
 const v2=pkg?.schemaVersion==='DIRECTOR_STANDARD_SCRIPT_SLICE_V2',v3=pkg?.schemaVersion==='DIRECTOR_STANDARD_SCRIPT_SLICE_V3',v4=pkg?.schemaVersion==='DIRECTOR_STANDARD_SCRIPT_SLICE_V4',v5=pkg?.schemaVersion==='DIRECTOR_STANDARD_SCRIPT_SLICE_V5',performance=v4||v5;
 check((v5||v4||v3||v2||pkg?.schemaVersion==='DIRECTOR_STANDARD_SCRIPT_SLICE_V1')&&pkg.fidelity?.receipt?.status==='MECHANICALLY_VERIFIED','STANDARD_PACKAGE','标准正文缺少机械保真回执');
 const {packageId,...withReceipt}=pkg,{receipt,...fidelity}=withReceipt.fidelity;
 const packageDigest=fingerprint({...withReceipt,fidelity});
 check(packageId===`standard:${packageDigest}`&&receipt.packageDigest===packageDigest&&receipt.sourceVersion===pkg.binding?.sourceVersion&&receipt.planDigest===pkg.binding?.planDigest,'STANDARD_RECEIPT','标准正文摘要与机械回执不一致');
 check(pkg.binding.sourceVersion===`src:${fingerprint([pkg.binding.sourceId,sourceText])}`,'STANDARD_SOURCE_BINDING','标准正文来源版本与当前原文不一致');
 check(receipt.schemaVersion===(v5?'STANDARD_SCRIPT_MECHANICAL_RECEIPT_V5':v4?'STANDARD_SCRIPT_MECHANICAL_RECEIPT_V4':v3?'STANDARD_SCRIPT_MECHANICAL_RECEIPT_V3':v2?'STANDARD_SCRIPT_MECHANICAL_RECEIPT_V2':'STANDARD_SCRIPT_MECHANICAL_RECEIPT_V1')&&['sourceId','sourceVersion','sourceIndexDigest','indexerVersion','planDigest'].every(field=>receipt[field]===pkg.binding[field])&&receipt.assemblerVersion===pkg.assemblerVersion&&pkg.assemblerVersion===(v5?'standard-script-compiler-v5':v4?'standard-script-compiler-v4':v3?'standard-script-compiler-v3':v2?'standard-script-compiler-v2':'standard-script-compiler-v1')&&pkg.binding.indexerVersion==='physical-lines-v1','STANDARD_RECEIPT','标准正文回执版本或来源绑定不一致');
 const grouped=(pkg.script?.entries||[]).some(e=>e.kind==='DIALOGUE_GROUP');
 const mixed=(pkg.script?.entries||[]).some(e=>e.kind==='BODY_GROUP');
 const postCues=(pkg.script?.entries||[]).some(e=>e.kind==='DIALOGUE_GROUP'?(Array.isArray(e.dialogues)?Array.from(e.dialogues).some(child=>postColonFields(sourceText,child)):false):e.kind==='DIALOGUE'&&postColonFields(sourceText,e));
 check((performance||mixed===v3&&(v3||(grouped||postCues)===v2))&&(!postCues||receipt.checks?.postColonCues==='VERIFIED')&&(!mixed||receipt.checks?.mixedBodyGroups==='VERIFIED')&&(!performance||receipt.checks?.inlinePerformance==='VERIFIED'),'STANDARD_RECEIPT','新版表演须V5，旧行中表演保留V4，混合正文须V3，既有对白分组/后置提示须V2，普通稿保留V1');
 const physical=[...sourceText.matchAll(/[^\r\n]*(?:\r\n|\r|\n|$)/g)].filter(match=>match[0].length).map((match,i)=>({id:`${pkg.binding.sourceVersion}:u${i+1}`,start:match.index,end:match.index+match[0].length,text:match[0]}));
 const sourceIndex={schemaVersion:'STANDARD_SCRIPT_SOURCE_INDEX_V1',indexerVersion:'physical-lines-v1',sourceId:pkg.binding.sourceId,sourceVersion:pkg.binding.sourceVersion,offsetUnit:'UTF16',text:sourceText,units:physical};
 check(fingerprint(sourceIndex)===pkg.binding.sourceIndexDigest,'STANDARD_RECEIPT','标准原文索引摘要不一致');
 const units=pkg.evidence;
 check(Array.isArray(units)&&units.length>0&&units.length===physical.length,'STANDARD_EVIDENCE','标准原文证据与真实物理行数量不一致');
 const byId=new Map();let cursor=0;
 for(const [i,unit] of units.entries()){
  check(unit&&['id','start','end','text'].every(field=>unit[field]===physical[i][field])&&unit.sourceVersion===pkg.binding.sourceVersion&&unit.offsetUnit==='UTF16','STANDARD_EVIDENCE','标准原文证据必须逐项对应真实物理行、来源版本与UTF16坐标');
  check(unit&&typeof unit.id==='string'&&!byId.has(unit.id)&&unit.start===cursor&&Number.isSafeInteger(unit.end)&&unit.end>unit.start&&typeof unit.text==='string'&&unit.end-unit.start===unit.text.length,'STANDARD_EVIDENCE','原文证据范围不连续或重复');
  byId.set(unit.id,{...unit,line:byId.size+1});cursor=unit.end;
 }
 check(units.map(u=>u.text).join('')===sourceText,'STANDARD_SOURCE_BINDING','标准正文与本集原文不一致');
 const entries=[...(pkg.script?.entries||[]),...(pkg.annotations||[]),...(pkg.exclusions||[]).map(e=>({...e,target:'EXCLUDE',sourceUnitIds:e.unitIds}))];
 const owned=new Set(),ids=new Set(),sourceRows=[],notes=[],annotations=[];let performanceCount=0,headCueCount=0;
 entries.sort((a,b)=>(byId.get(a.sourceUnitIds?.[0])?.start??-1)-(byId.get(b.sourceUnitIds?.[0])?.start??-1));
 for(const entry of entries){
  check(typeof entry.id==='string'&&!ids.has(entry.id)&&Array.isArray(entry.sourceUnitIds)&&entry.sourceUnitIds.length>0,'STANDARD_ENTRY','标准节点缺少唯一ID或来源');ids.add(entry.id);
  const selected=entry.sourceUnitIds.map(id=>byId.get(id));
  check(selected.every(Boolean),'STANDARD_ENTRY','标准节点引用未知原文');
  for(let i=0;i<selected.length;i++){const unit=selected[i];check(!owned.has(unit.id)&&(!i||selected[i-1].end===unit.start),'STANDARD_ENTRY','原文重复归属或节点不连续');owned.add(unit.id);}
  const start=selected[0].start,end=selected.at(-1).end,text=sourceText.slice(start,end),spans=[];
  const performanceField=child=>{
   const headCue=v5?compactPerformanceHeadProjection(sourceText,child.speaker?.parts,child.cue?.parts,child.channelSource?.parts):null;if(headCue)headCueCount++;
   const expected=performance?inlinePerformanceProjection(sourceText,child.textAssembly?.parts,v5?'inline-performance-v2':'inline-performance-v1'):null;
   check(expected?child.performance!==undefined&&fingerprint(child.performance)===fingerprint(expected):child.performance===undefined,'STANDARD_PERFORMANCE_BINDING','行中表演、发声片段及位置必须由原文精确派生，不能遗漏、改写或新增');
   if(!expected)return null;performanceCount++;
   const local=parts=>parts.map(p=>[p.start-start,p.end-start]);
   return {spoken:local(expected.spoken.parts),cues:expected.cues.map(c=>({text:c.text,parts:local(c.parts),markerParts:local(c.markerParts),quoteOffset:c.spokenOffset}))};
  };
  const parts=(raw,expected,label,empty=false,sink=spans,captionGaps=false)=>{
   check(Array.isArray(raw)&&(empty||raw.length>0),'STANDARD_PARTS',`${label}缺少来源片段`);
   const isLayout=label.endsWith('版式');
   let previous=-1;
   const result=raw.map(part=>{
    const unit=byId.get(part.unitId);
    check(unit&&entry.sourceUnitIds.includes(part.unitId)&&Number.isSafeInteger(part.start)&&Number.isSafeInteger(part.end)&&part.start>=unit.start&&part.end<=unit.end&&part.end>part.start,'STANDARD_PARTS',`${label}原文范围无效`);
    check(['SOURCE_SLICE','SOURCE_SEPARATOR'].includes(part.kind)&&(part.kind!=='SOURCE_SEPARATOR'||/^[\t \r\n]+$/.test(sourceText.slice(part.start,part.end))),'STANDARD_PARTS','标准片段类型不合法');
    check(part.start>=previous&&(isLayout||captionGaps||previous<0||part.start===previous),'STANDARD_PARTS','标准正文切片不连续');previous=part.end;
    if(isLayout)check(/^[\s\uFEFF:：()（）△▲【】\[\],，]+$/.test(sourceText.slice(part.start,part.end)),'STANDARD_LAYOUT','标准版式不能隐藏正文字符');
    if(v2||v3||performance)for(const offset of [part.start,part.end])check(!(offset>0&&offset<sourceText.length&&/[\uD800-\uDBFF]/.test(sourceText[offset-1])&&/[\uDC00-\uDFFF]/.test(sourceText[offset])),'STANDARD_PARTS','对白分组不能切开Unicode字符');
    sink.push([part.start,part.end]);return [part.start-start,part.end-start];
   });
   check(result.map(([a,b])=>text.slice(a,b)).join('')===expected,'STANDARD_PARTS',`${label}不是原文精确切片`);
   return result;
  };
  const id=`${key}:${entry.id}`;
  if(entry.target==='EXCLUDE'){
   check(entry.category==='BLANK'&&/^\s*$/.test(text),'STANDARD_EXCLUSION','非空原文不可排除出制作范围');
   sourceRows.push({id,line:selected[0].line,text,standard:{target:'EXCLUDE',kind:'BLANK'}});continue;
  }
  check(['BODY','STRUCTURE','ANNOTATION'].includes(entry.target),'STANDARD_ENTRY','未知标准节点分型');
  const standard={target:entry.target,kind:entry.kind,parts:parts(entry.textAssembly?.parts,entry.text,'正文')};
  const layout=entry.layout||[];parts(layout,layout.map(part=>sourceText.slice(part.start,part.end)).join(''),'版式',true);
  if(entry.kind==='DIALOGUE'){
   check(entry.target==='BODY'&&Object.hasOwn(voices,entry.channel),'STANDARD_CHANNEL','对白声道或节点类型无效');
   standard.speaker=parts(entry.speaker?.parts,entry.speaker?.text,'说者');
   standard.cue=entry.cue?parts(entry.cue.parts,entry.cue.text,'表演提示'):[];
   standard.channelSource=entry.channelSource?parts(entry.channelSource.parts,entry.channelSource.text,'声道'):[];
   standard.channel=voices[entry.channel];
   compactVoice(sourceText,entry,v5);
   if(postColonFields(sourceText,entry)){postCueHeader(sourceText,start,entry.textAssembly.parts[0].start,entry);groupUtterance(entry.text,entry.speaker.text,entry.cue?.text);}
   const projection=performanceField(entry);if(projection)standard.performance=projection;
  }
  if(entry.kind==='BODY_GROUP'){
   check((v3||performance)&&entry.target==='BODY'&&entry.text===text&&layout.length===0&&Array.isArray(entry.children)&&entry.children.length>=2&&entry.children[0]?.kind==='DIALOGUE'&&entry.children.slice(1).every(c=>c?.kind==='ONSCREEN_TEXT'),'STANDARD_BODY_GROUP','混合正文须保留完整源范围、一个对白和显式字幕');
   const childSpans=[],captions=[];
   standard.children=entry.children.map((child,i)=>{
    mixedChildShape(child,performance);
    const isDialogue=i===0;
    const own=[],body=parts(child.textAssembly?.parts,child.text,'混合正文',false,own,isDialogue);
    check(child.text.trim(),'STANDARD_BODY_GROUP','混合正文不能含空对白或空字幕');
    parts(child.layout,child.layout.map(p=>sourceText.slice(p.start,p.end)).join(''),'混合版式',true,own);
    let result;
    if(isDialogue){
     check(Object.hasOwn(voices,child.channel),'STANDARD_CHANNEL','混合对白声道无效');
     const speaker=parts(child.speaker?.parts,child.speaker?.text,'混合说者',false,own),cue=child.cue?parts(child.cue.parts,child.cue.text,'混合提示',false,own):[],channelSource=child.channelSource?parts(child.channelSource.parts,child.channelSource.text,'混合声道',false,own):[];
     compactVoice(sourceText,child,v5);const projection=performanceField(child);result={kind:'DIALOGUE',parts:body,speaker,cue,channelSource,channel:voices[child.channel],...(projection?{performance:projection}:{})};
    }else{
     const label=parts(child.label?.parts,child.label?.text,'字幕标记',false,own);
     own.sort((a,b)=>a[0]-b[0]);let edge=own[0][0];
     for(const [a,b] of own){check(a===edge,'STANDARD_BODY_COVERAGE','字幕范围漏字或重叠');edge=b;}
     captions.push({start:own[0][0],end:edge,child});result={kind:'ONSCREEN_TEXT',parts:body,label};
    }
    childSpans.push(...own);return result;
   });
   childSpans.sort((a,b)=>a[0]-b[0]);let edge=start;
   for(const [a,b] of childSpans){check(a===edge,'STANDARD_BODY_COVERAGE','混合子节点漏字或重复');edge=b;}
   check(edge===end,'STANDARD_BODY_COVERAGE','混合子节点未覆盖完整来源');
   const d=entry.children[0],a=Math.min(d.textAssembly.parts[0].start,captions[0].start),b=Math.max(d.textAssembly.parts.at(-1).end,captions.at(-1).end),raw=sourceText.slice(a,b),markers=subtitleSpans(raw);
   check(markers.length===captions.length&&/^\s*$/.test(sourceText.slice(b,end)),'STANDARD_BODY_CAPTION','字幕目录不等于源文显式标记');
   let spoken='',cursor=0;
   for(const [i,m] of markers.entries()){
    const c=captions[i],exact=(field,from,to)=>field.parts[0].start===a+from&&field.parts.at(-1).end===a+to;
    check(c.start===a+m.start&&c.end===a+m.end&&exact(c.child.label,m.labelStart,m.labelEnd)&&exact({parts:c.child.textAssembly.parts},m.bodyStart,m.bodyEnd),'STANDARD_BODY_CAPTION','字幕内容/标记不等于原文精确范围');
    spoken+=raw.slice(cursor,m.start);cursor=m.end;
   }
   spoken+=raw.slice(cursor);check(spoken===d.text,'STANDARD_BODY_SPEECH','对白只能是原范围去除明确字幕后的逐字内容');
   const headerChild={...d,textAssembly:{parts:[{...d.textAssembly.parts[0],start:a}]}};
   if(postColonFields(sourceText,headerChild))postCueHeader(sourceText,start,a,d);
   else{
    const compact=compactVoice(sourceText,d,v5),header=/^\s*([^:：()（）]+?)(?:[（(]([^()（）]*)[）)])?(?:(?<=[）)])[\t \u00a0\u3000]*(OS|VO))?[ \t]*[:：][ \t\r\n]*$/iu.exec(sourceText.slice(start,a));
    const qualified=Boolean(header?.[2]&&!/^(?:OS|VO|旁白|画外音|内心独白|系统音)$/iu.test(header[2].trim())&&sourceText.slice(start,a).replace(/(?<=[）)])[\t \u00a0\u3000]*(?:OS|VO)[\t \u00a0\u3000]*(?=[:：])/iu,'').replace(/[:：][\s]*$/u,'').trim()===d.speaker.text&&declaredSpeakerForm(sourceText,d.speaker.text));
    check(header&&(qualified||header[1].trim()===sourceHeadPrefix(sourceText,d.speaker,d.cue,d.channelSource)),'STANDARD_BODY_HEADER','混合对白说者与原文头不一致');
    const fields=[d.speaker,d.cue,d.channelSource].filter(Boolean).sort((x,y)=>x.parts[0].start-y.parts[0].start);let edge=start;
    for(const f of fields){check(f.parts[0].start>=edge&&(edge===start?/^[\s\uFEFF]*$/:/^[\s()（）,，]*$/).test(sourceText.slice(edge,f.parts[0].start)),'STANDARD_BODY_HEADER','混合对白字段跨越源头');edge=f.parts.at(-1).end;}
    check(edge<=a&&/^[\t ()（）]*[:：][\t \r\n]*$/.test(sourceText.slice(edge,a)),'STANDARD_BODY_HEADER','混合对白冒号边界无效');
    const tokens=[...(compact&&!header[3]?[compact.text]:[]),...(header[3]?[header[3]]:[]),...(qualified?'':header[2]??'').split(/[,， \t]+/).filter(Boolean)],channels=tokens.filter(t=>/^(OS|VO)$/i.test(t)),expected=channels[0]?.toUpperCase()??({旁白:'NARRATION',画外音:'VO',系统音:'SYSTEM'})[d.speaker.text]??'SPEECH';
    check(channels.length<=1&&d.channel===expected&&(channels.length?d.channelSource?.text===channels[0]:!d.channelSource),'STANDARD_BODY_CHANNEL','混合对白声道与源文不一致');
   }
   groupUtterance(d.text,d.speaker.text,d.cue?.text);
  }
  if(entry.kind==='DIALOGUE_GROUP'){
   check(entry.target==='BODY'&&selected.length===1&&entry.text===text&&layout.length===0&&Array.isArray(entry.dialogues)&&entry.dialogues.length>=2,'STANDARD_GROUP','对白分组须保留完整物理行和至少两段对白');
   let groupEdge=start;
   standard.dialogues=Array.from(entry.dialogues,(child,i)=>{
    check(child&&Object.keys(child).every(k=>['text','textAssembly','layout','speaker','channel','channelSource','cue',...(performance?['performance']:[])].includes(k)),'STANDARD_GROUP','对白子目录结构无效');
    const childSpans=[],body=parts(child.textAssembly?.parts,child.text,'组正文',false,childSpans),speaker=parts(child.speaker?.parts,child.speaker?.text,'组说者',false,childSpans);
    const cue=child.cue?parts(child.cue.parts,child.cue.text,'组提示',false,childSpans):[],channelSource=child.channelSource?parts(child.channelSource.parts,child.channelSource.text,'组声道',false,childSpans):[];
    const compact=compactVoice(sourceText,child,v5);
    check(Array.isArray(child.layout),'STANDARD_GROUP','对白分组缺少版式');
    parts(child.layout,child.layout.map(p=>sourceText.slice(p.start,p.end)).join(''),'组版式',true,childSpans);
    childSpans.sort((a,b)=>a[0]-b[0]);const childStart=groupEdge;
    for(const [a,b] of childSpans){check(a===groupEdge,'STANDARD_GROUP_COVERAGE','对白子目录漏字、重复或顺序错误');groupEdge=b;}
    const a=body[0][0]+start,b=body.at(-1)[1]+start;
    check(child.text.trim()&&/^[\t \r\n]*$/.test(sourceText.slice(b,groupEdge)),'STANDARD_GROUP_BOUNDARY','对白子目录不能隐藏尾部正文');
    if(postColonFields(sourceText,child))postCueHeader(sourceText,childStart,a,child);
    else{
    const header=/^\s*([^:：()（）]+?)(?:[（(]([^()（）]*)[）)])?(?:(?<=[）)])[\t \u00a0\u3000]*(OS|VO))?[ \t]*[:：][ \t]*$/iu.exec(sourceText.slice(childStart,a));
    const qualified=Boolean(header?.[2]&&!/^(?:OS|VO|旁白|画外音|内心独白|系统音)$/iu.test(header[2].trim())&&sourceText.slice(childStart,a).replace(/(?<=[）)])[\t \u00a0\u3000]*(?:OS|VO)[\t \u00a0\u3000]*(?=[:：])/iu,'').replace(/[:：][ \t]*$/u,'').trim()===child.speaker.text&&declaredSpeakerForm(sourceText,child.speaker.text));
    check(header&&(qualified||header[1].trim()===sourceHeadPrefix(sourceText,child.speaker,child.cue,child.channelSource))&&/^[\p{L}\p{N}·・ _.-]+(?:、[\p{L}\p{N}·・ _.-]+)*$/u.test(qualified?header[1].trim():child.speaker.text),'STANDARD_GROUP_HEADER','对白分组说者与原文头不一致');
    const fields=[speaker,cue,channelSource].filter(f=>f.length).sort((x,y)=>x[0][0]-y[0][0]);let edge=childStart;
    for(const f of fields){check(f[0][0]+start>=edge&&(edge===childStart?/^[\s\uFEFF()（）【】\[\]]*$/:/^[\s\uFEFF()（）【】\[\],，]*$/).test(sourceText.slice(edge,f[0][0]+start)),'STANDARD_GROUP_HEADER','对白分组字段跨过原文头');edge=f.at(-1)[1]+start;}
    check(edge<=a&&/^[\t ()（）【】\[\]]*[:：][\t ]*$/.test(sourceText.slice(edge,a)),'STANDARD_GROUP_HEADER','对白分组冒号边界无效');
    const tokens=[...(compact&&!header[3]?[compact.text]:[]),...(header[3]?[header[3]]:[]),...(qualified?'':header[2]||'').split(/[,， \t]+/).filter(Boolean)],channels=tokens.filter(token=>/^(OS|VO)$/i.test(token));
    const expected=channels.length?channels[0].toUpperCase():({旁白:'NARRATION',画外音:'VO',系统音:'SYSTEM'})[child.speaker.text]||'SPEECH';
    check(channels.length<=1&&child.channel===expected&&(channels.length?child.channelSource?.text===channels[0]:!child.channelSource),'STANDARD_GROUP_CHANNEL','对白分组声道与原文不一致');
    }
    if(i)check(/(?:[。.!！？?;；,，…][”’」』"']?|[ \t])$/u.test(sourceText.slice(start,speaker[0][0]+start)),'STANDARD_GROUP_BOUNDARY','对白分组间缺少明确分隔边界');
    groupUtterance(child.text,child.speaker.text,child.cue?.text);
    const projection=performanceField(child);return {parts:body,speaker,cue,channelSource,channel:voices[child.channel],...(projection?{performance:projection}:{})};
   });
   check(groupEdge===end,'STANDARD_GROUP_COVERAGE','对白子目录未覆盖完整原行');
  }
  spans.sort((a,b)=>a[0]-b[0]);let edge=start;
  for(const [a,b] of spans){check(a===edge,'STANDARD_COVERAGE','标准节点内部漏字或重复');edge=b;}
  check(edge===end,'STANDARD_COVERAGE','标准节点未覆盖完整原文');
  if(entry.target==='ANNOTATION'){
   notes.push({id,text,evidenceRole:'guidance'});annotations.push({id,kind:entry.kind});
  }else sourceRows.push({id,line:selected[0].line,text,standard});
 }
 check(owned.size===units.length,'STANDARD_COVERAGE','标准正文仍有未归属原文');
 check(!performance||performanceCount>0||v5&&headCueCount>0,'STANDARD_PERFORMANCE_BINDING','V4/V5须实际包含有来源的表演分层');
 check(sourceRows.some(r=>r.standard.target==='BODY'&&r.text.trim()),'SOURCE_EMPTY','本集没有可制作正文');
 return {key,title,sourceProtocol:STANDARD_EPISODE_PROTOCOL,standardPackage:pkg,context:{...context,...(annotations.length?{annotations,annotationRule:'annotations是有来源的辅助说明，不是新增剧情或已证实事实；不得据此虚构entryFacts。'}:{})},contextRows:[...contextRows,...notes],sourceRows};
}

/** Saved/sliced rows must retain their canonical proof. This proof is never added to modelSource. */
export function verifyStandardSource(source){
 const pkg=source.standardPackage;
 check(pkg&&Array.isArray(pkg.evidence),'STANDARD_RECEIPT','标准源缺少已绑定的保真包');
 const original=standardPackageSource(pkg,{key:String(source.key),title:source.title,context:{},sourceText:pkg.evidence.map(unit=>unit.text).join('')});
 const expected=new Map([...original.sourceRows,...original.contextRows].map(row=>[row.id,row]));
 const allIds=original.sourceRows.map(row=>row.id),scope=source.standardScope;
 if(scope)check(scope.version==='standard-scope-v1'&&Array.isArray(scope.sourceIds)&&new Set(scope.sourceIds).size===scope.sourceIds.length&&scope.sourceIds.every(id=>allIds.includes(id)),'STANDARD_SCOPE','标准切片范围无效');
 const selected=scope?allIds.filter(id=>scope.sourceIds.includes(id)):allIds;
 check(JSON.stringify((source.sourceRows||[]).map(row=>row.id))===JSON.stringify(selected),'STANDARD_SOURCE_COVERAGE','标准正文节点覆盖或顺序不一致');
 check(original.contextRows.every(note=>(source.contextRows||[]).some(row=>row.id===note.id)),'STANDARD_NOTE_COVERAGE','标准辅助资料缺失，不能静默省略');
 for(const row of [...(source.sourceRows||[]),...(source.contextRows||[])]){
  const bound=expected.get(row.id);
  if(!bound){check(!row.standard,'STANDARD_ROW_BINDING','标准节点不属于本集源包');continue;}
  check(row.text===bound.text&&row.line===bound.line&&fingerprint(row.standard??null)===fingerprint(bound.standard??null)&&row.evidenceRole===bound.evidenceRole,'STANDARD_ROW_BINDING','标准源行与保真包绑定不一致');
 }
 for(const row of source.sourceRows||[])check(expected.has(row.id)&&row.standard,'STANDARD_ROW_BINDING','制作源行缺少标准归属证明');
}

export function standardRowKind(row){
 const s=row.standard;
 check(s&&['BODY','STRUCTURE','EXCLUDE'].includes(s.target),'STANDARD_ROW','标准源行缺少分型');
 if(s.target==='EXCLUDE'){check(/^\s*$/.test(row.text),'STANDARD_EXCLUSION','非空源行不可隐藏');return 'blank';}
 if(s.target==='STRUCTURE')return 'metadata';
 check(['ACTION','DIALOGUE','DIALOGUE_GROUP','BODY_GROUP','SFX','ONSCREEN_TEXT'].includes(s.kind),'STANDARD_ROW','未知正文类型');
 return ['DIALOGUE','DIALOGUE_GROUP','BODY_GROUP'].includes(s.kind)?'dialogue':'action';
}
export function standardDialogues(row){
 if(standardRowKind(row)!=='dialogue')return [];
 const children=row.standard.kind==='BODY_GROUP'?row.standard.children.filter(c=>c.kind==='DIALOGUE'):row.standard.kind==='DIALOGUE_GROUP'?row.standard.dialogues:[row.standard];
 check(Array.isArray(children)&&children.length>0,'STANDARD_ROW','对白子目录缺失');
 return Array.from(children,(s,i)=>{
 const copy=spans=>{check(Array.isArray(spans),'STANDARD_ROW','缺少来源切片');let edge=-1;return spans.map(pair=>{check(Array.isArray(pair)&&pair.length===2&&pair.every(Number.isSafeInteger)&&pair[0]>=0&&pair[1]>pair[0]&&pair[1]<=row.text.length&&pair[0]>=edge,'STANDARD_ROW','对白来源范围无效');edge=pair[1];return row.text.slice(...pair);}).join('');};
 const speaker=copy(s.speaker),quote=copy(s.performance?.spoken??s.parts),cue=[copy(s.channelSource||[]),copy(s.cue||[])].filter(Boolean).join('；');
 check(speaker.trim()&&quote.trim()&&Object.values(voices).includes(s.channel),'STANDARD_ROW','缺少说者、正文或声道');
 return {id:`${row.id}:D${i+1}`,line:row.line,speaker,quote,cue,channel:s.channel,rowId:row.id,...(s.performance?{performanceCues:structuredClone(s.performance.cues)}:{})};
 });
}
export function standardDialogue(row,index=1){
 return standardDialogues(row)[row.standard.kind==='DIALOGUE_GROUP'?index-1:0]??null;
}

export function standardOnscreenTexts(row){
 if(row.standard?.kind==='ONSCREEN_TEXT'){
  const all=row.standard.parts.map(pair=>[...pair]),raw=all.map(([a,b])=>row.text.slice(a,b)).join('');
  // The typed label remains in the proof/source row, not the visible caption.
  const prefix=/^(?:弹幕|字幕|屏幕文字)[\t \u00a0\u3000]*[:：][\t \u00a0\u3000]*/u.exec(raw)?.[0].length??0;
  let edge=0;const parts=all.flatMap(([a,b])=>{const skip=Math.max(0,prefix-edge);edge+=b-a;return a+skip<b?[[a+skip,b]]:[];});
  return [{id:`${row.id}:T1`,rowId:row.id,line:row.line,kind:'ONSCREEN_TEXT',text:raw.slice(prefix),parts,sourceIds:[row.id]}];
 }
 if(row.standard?.kind!=='BODY_GROUP')return [];
 return row.standard.children.filter(c=>c.kind==='ONSCREEN_TEXT').map((child,i)=>({id:`${row.id}:T${i+1}`,rowId:row.id,line:row.line+(row.text.slice(0,child.parts[0][0]).match(/\r\n|\r|\n/g)?.length??0),kind:'ONSCREEN_TEXT',text:child.parts.map(([a,b])=>row.text.slice(a,b)).join(''),parts:child.parts.map(pair=>[...pair]),sourceIds:[row.id]}));
}
