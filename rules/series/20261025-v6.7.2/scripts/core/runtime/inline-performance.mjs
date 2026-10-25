// This is a conservative syntax view, not rewriting or an AI judgement.
// Uncertain/explanatory/nested/quoted parentheses remain protected speech.
export const INLINE_PERFORMANCE_VERSION='inline-performance-v2';
const acting=/^(?:低声|轻声|小声|大声|压低声音|提高声音|压住颤音|哽咽|哭腔|带哭腔|气喘|喘息|咬牙|大笑|苦笑|冷笑|轻笑|笑着|哭着|怒视|皱眉|挑眉|点头|摇头|抬头|低头|转头|转身|起身|蹲下|抬手|摆手|挥手|握拳|攥拳|摊手|耸肩|抿嘴|闭眼|睁眼|深吸一口气|叹气|掐着(?:手指|指头))$/u;
// V1 is immutable: existing plans, packages and paid requests rederive its exact view.
export const explicitPerformanceCue=(value,version=INLINE_PERFORMANCE_VERSION)=>value.trim().split(/[,，、;；]/u).every(x=>acting.test(x.trim())||version==='inline-performance-v2'&&/^(?:恍然大悟|恍然大悟的样子)$/u.test(x.trim()));
/** OS/VO is a source field, not a name suffix. Latin names require an explicit gap. */
export function sourceVoiceHead(text){
 const match=/^(.+?)(OS|VO)$/iu.exec(text);
 if(!match||!/(?:\p{Script=Han}|[\t \u00a0\u3000])$/u.test(match[1]))return null;
 const speaker=match[1].trimEnd();
 return speaker?{speaker,text:match[2],start:text.length-2,end:text.length}:null;
}
export function sourceVoiceProjection(source,speaker){
 if(!speaker?.parts?.length)return null;
 const end=speaker.parts.at(-1).end,rest=source.slice(end);
 const match=/^([\t \u00a0\u3000]*)(OS|VO)(?=[\t \u00a0\u3000]*(?:[（(:：]))/iu.exec(rest)
  ??/^([\t \u00a0\u3000]*[（(][^()（）\r\n]*[）)][\t \u00a0\u3000]*)(OS|VO)(?=[\t \u00a0\u3000]*[:：])/iu.exec(rest);
 if(!match||!match[1]&&!/(?:\p{Script=Han}|[）)])$/u.test(speaker.text))return null;
 const start=end+match[1].length;
 return {text:match[2],start,end:start+2};
}
// Speech introducers only. This does not expand the immutable inline-cue view.
// The parser separately requires independent actor identity, not a guessed alias.
export function sourceHeadPerformanceCue(value){
 const text=value.trim();
 return explicitPerformanceCue(text)||/^(?:(?:微微|轻轻|忽然|突然|忍不住)?(?:憋笑|怒吼|吼道|笑道|听完|听罢|精神传音)|(?:声音|嗓音)(?:微微|略微|不住地)?(?:发抖|颤抖|沙哑)|笑容(?:瞬间|突然|渐渐)?(?:僵住|僵死|消失|凝固)|(?:急|气|笑|哭)得(?:眼圈发红|满脸通红|浑身发抖|直跺脚|说不出话)|(?:这时|此时|突然)?(?:再次)?(?:提示|提醒|通知))$/u.test(text);
}
export function sourceBarePerformanceProjection(source,speaker,cue){
 if(!speaker?.parts?.length||!cue?.parts?.length||!sourceHeadPerformanceCue(cue.text))return null;
 const a=speaker.parts.at(-1).end,b=cue.parts[0].start;
 if(b<a||!/^[\t \u00a0\u3000]*$/u.test(source.slice(a,b)))return null;
 return {start:b,end:cue.parts.at(-1).end};
}
/** Literal prefix before parenthesized metadata; source fields stay byte-bound. */
export function sourceHeadPrefix(source,speaker,cue,channelSource){
 let end=speaker.parts.at(-1).end;
 const bare=sourceBarePerformanceProjection(source,speaker,cue);if(bare)end=bare.end;
 if(channelSource?.parts?.length){const a=channelSource.parts[0].start,b=channelSource.parts.at(-1).end;
  if(a>=end&&/^[\t \u00a0\u3000]*$/u.test(source.slice(end,a))&&/^(OS|VO)$/iu.test(channelSource.text))end=b;
 }
 return source.slice(speaker.parts[0].start,end);
}
// Explicit explanation predicates, system properties and voiced clause
// readings are not second speakers. Arbitrary labels do not gain this exception.
// Keep the colon and value in the original utterance, never as a world rule.
export function sourceUtteranceProperty(speaker,label,cue=''){
 const read=/(?:念出(?:了)?声(?:来)?|读出(?:了)?声(?:来)?|朗读|宣读|(?:大声|高声|低声|轻声|开口)(?:念着|读着))/u;
 const silent=/(?:不(?:要|许|能|会|敢|再|想)?|没(?:有)?|(?:尚|还)?未(?:曾)?|莫|别|禁止|默(?:默)?)[^，,；;。.!！？?]{0,12}(?:念出|念着|朗读|宣读|读出|读着)|默读|默念|脑海|心中|内心|心里|无声|不出声|没(?:有)?(?:出声|发出)|声音[^，,；;。.!！？?]{0,8}(?:没|未|不)/u;
 // A finite explanation predicate is spoken syntax, not a character or a
 // world rule. All original words and the colon remain in the utterance.
 return /^[\p{L}\p{N}]{1,16}(?:老样子|如下|很简单|是这样)$/u.test(label)
  || /^(?:系统|系统音|系统窗口|系统提示窗口)$/u.test(speaker)&&/^(?:触发(?:终极)?(?:主线|支线)?任务|任务目标|任务奖励|解锁(?:全新)?(?:建筑|技能))$/u.test(label)
  || read.test(cue)&&!silent.test(cue)&&/^(?:条款|条目)[一二三四五六七八九十百\d]+$/u.test(label);
}
/** Mechanical header shape only; the parser separately requires independent identity evidence. */
export function compactPerformanceHeadProjection(source,speaker,cue,channelSource){
 if(!speaker?.length||!cue?.length||!channelSource?.length)return null;
 const copy=parts=>parts.map(p=>source.slice(p.start,p.end)).join('');
 const cueText=copy(cue),text=copy(channelSource),start=channelSource[0].start,end=channelSource.at(-1).end;
 if(!/\p{Script=Han}$/u.test(copy(speaker))||speaker.at(-1).end!==cue[0].start||!/^[\t \u00a0\u3000]*$/u.test(source.slice(cue.at(-1).end,start))||cue.at(-1).end>start
  ||!sourceHeadPerformanceCue(cueText)||!/^(OS|VO)$/iu.test(text)||end-start!==2||!/^[\t ]*(?:[（(:：])/u.test(source.slice(end)))return null;
 return {text,start,end,cueText};
}
const literalMention=(text,start,end)=>/(?:叫|名为|称作|写着|写成|写的是|读作|读成|念作|念成|字样[：:]?)\s*$/u.test(text.slice(0,start))
 || /^\s*(?:这(?:个|两个|几个)?字|这些字|这个词|这个字样|一词|二字|两个字|三个字)/u.test(text.slice(end));

/** All slices refer to the original source, not a new rewritten script. */
export function inlinePerformanceProjection(source,parts,version=INLINE_PERFORMANCE_VERSION){
 if(!['inline-performance-v1','inline-performance-v2'].includes(version))throw new Error('Unsupported inline performance version');
 if(!Array.isArray(parts)||!parts.length)return null;
 let size=0;
 const mapped=parts.map(part=>{const from=size;size+=part.end-part.start;return {part,from,to:size};});
 const text=parts.map(p=>source.slice(p.start,p.end)).join('');
 const slice=(start,end)=>mapped.flatMap(({part,from,to})=>{
  const a=Math.max(from,start),b=Math.min(to,end);
  return a<b?[{...part,start:part.start+a-from,end:part.start+b-from}]:[];
 });
 const markers=[],stack=[],pairs={'“':'”','‘':'’','「':'」','『':'』','"':'"',"'":"'",'【':'】','[':']'};
 for(let i=0;i<text.length;i++){
  const c=text[i];if(c==='\\'){i++;continue;}
  if((c==="'"||c==='’')&&/[A-Za-z0-9]/.test(text[i-1]??'')&&/[A-Za-z0-9]/.test(text[i+1]??''))continue;
  if(stack.at(-1)===c){stack.pop();continue;}
  if(c==="'"&&!stack.includes("'")&&/[sS]/.test(text[i-1]??'')&&/\s/.test(text[i+1]??''))continue;
  if(pairs[c]){stack.push(pairs[c]);continue;}
  if(stack.length||!/[（(]/u.test(c))continue;
  // Balance the whole group first: do not discover inner cues inside an uncertain outer group.
  const brackets=[c==='（'?'）':')'];let end=i+1;
  for(;end<text.length&&brackets.length;end++){
   if(/[（(]/u.test(text[end]))brackets.push(text[end]==='（'?'）':')');
   else if(/[）)]/u.test(text[end])){if(brackets.at(-1)!==text[end])break;brackets.pop();}
  }
  if(brackets.length){i=text.length;continue;}
  const value=text.slice(i+1,end-1),markerParts=slice(i,end);
  const contiguous=markerParts.every((p,j)=>!j||markerParts[j-1].end===p.start);
  if(contiguous&&explicitPerformanceCue(value,version)&&!literalMention(text,i,end))markers.push({start:i,end,value,markerParts});
  i=end-1;
 }
 if(!markers.length)return null;
 const spokenParts=[],cues=[];let cursor=0,spokenOffset=0;
 for(const marker of markers){
  spokenParts.push(...slice(cursor,marker.start));spokenOffset+=marker.start-cursor;
  const start=marker.start+1+marker.value.length-marker.value.trimStart().length,end=marker.end-1-(marker.value.length-marker.value.trimEnd().length);
  cues.push({text:marker.value.trim(),parts:slice(start,end),markerParts:marker.markerParts,spokenOffset});cursor=marker.end;
 }
 spokenParts.push(...slice(cursor,text.length));
 const spoken=spokenParts.map(p=>source.slice(p.start,p.end)).join('');
 if(!spoken.trim())return null;
 return {version,spoken:{text:spoken,parts:spokenParts},cues};
}
