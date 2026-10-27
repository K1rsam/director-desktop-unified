import {fail} from './shared.mjs';
import {STANDARD_EPISODE_PROTOCOL,standardRowKind,standardDialogue,standardDialogues,standardOnscreenTexts,verifyStandardSource} from './standard-source.mjs';

const pad=n=>`L${String(n).padStart(4,'0')}`;
const present=x=>x!==undefined&&x!==null&&x!=='';
function rowId(key,row){
  if(present(row.id))return String(row.id);
  if(present(row.sourceId))return `${key}:${row.sourceId}`;
  if(Number.isInteger(row.line))return `${key}:${pad(row.line)}`;
  fail('SOURCE_ROW_COORDINATE','源行缺少稳定坐标');
}
function aliases(row){
  const a=[row.id,row.sourceId];
  if(Number.isInteger(row.line)&&row.numericLineAlias!==false)a.push(String(row.line));
  if(Number.isInteger(row.line)&&!/^P\d+$/i.test(row.sourceId||''))a.push(pad(row.line),`L${row.line}`);
  if(Number.isInteger(row.sourceFileLine))a.push(pad(row.sourceFileLine),`L${row.sourceFileLine}`);
  return [...new Set(a.filter(present).map(String))];
}
function locate(rows,ref,sourceKey){
  const key=String(ref);
  const hits=rows.filter(row=>aliases(row).some(a=>a===key||`${sourceKey}:${a}`===key));
  if(hits.length>1)fail('SOURCE_REF_AMBIGUOUS',`源引用歧义: ${key}`);
  if(!hits.length)fail('SOURCE_REF_UNKNOWN',`源引用不存在: ${key}`);
  return hits[0];
}
export function resolveSourceRef(source,ref,{allowContext=false}={}){
  const rows=[...(source.sourceRows||[]),...(allowContext?source.contextRows||[]:[])];
  return locate(rows,ref,source.key);
}
function normalizeRows(key,rows){return (rows||[]).map(row=>{
  if(typeof row?.text!=='string')fail('SOURCE_ROW_TEXT','源行缺少原文text');
  if(present(row.id)&&typeof row.id!=='string'||present(row.sourceId)&&typeof row.sourceId!=='string'||present(row.line)&&(!Number.isInteger(row.line)||row.line<0)||present(row.sourceFileLine)&&(!Number.isInteger(row.sourceFileLine)||row.sourceFileLine<0))fail('SOURCE_ROW_COORDINATE','源行坐标类型无效');
  return {...row,id:rowId(key,row)};
});}
function checkAliases(rows,key){
  const owners=new Map();
  for(const row of rows)for(const alias of aliases(row)){
    if(owners.has(alias))fail('SOURCE_ALIAS_AMBIGUOUS',`源引用歧义或重复: ${alias}`);
    owners.set(alias,row.id);
  }
}
function channelOf(d){
  const label=[d.channel,d.cue].filter(Boolean).join(' ');
  if(/画外咆哮/.test(label))return '现场对白';
  if(['现场对白','VO','OS','系统音'].includes(d.channel))return d.channel;
  if(/系统音/.test(label)||d.speaker==='系统音')return '系统音';
  if(/(?:^|\W)VO(?:\W|$)/i.test(label))return 'VO';
  if(/(?:^|\W)OS(?:\W|$)/i.test(label))return 'OS';
  return d.channel||'现场对白';
}
function kindOf(d){
  if(d.kind)return d.kind;
  const cue=[d.cue,d.channel].filter(Boolean).join(' ');
  const pure=/^[啊呀哦喔呃呜吼嗷哇唔哼哈！!？?，,。.…\s]+$/.test(d.quote||'');
  if(pure&&/咆哮|兽吼|怒吼/.test(cue))return 'roar';
  if(pure&&/尖叫|惨叫/.test(cue))return 'scream';
  if(pure&&/哼|呜咽|呻吟/.test(cue))return 'vocalization';
  return 'speech';
}
function metadata(text,legacy=false){
  if(/^\s*(?:\d+[—–-]\d+(?:\s|[：:])|出场人物[：:]|第.{0,12}集(?:[：:]|\s*$)|[一二三四五六七八九十]+[：:]|(?:标题|场景|时间|地点)[：:]|场景[ \t　]+[^\s：:][^：:\r\n]*$)/.test(text))return true;
  if(legacy)return false;
  if(/^\s*(?:人物|角色|道具|天气|内外|时间层)\s*[：:]/.test(text))return true;
  const numbered=text.match(/^\s*(?:场\s*)?\d+\s*([—–－.．-])\s*\d+(?!\d|[.．])([^\r\n：:]*)$/);
  if(numbered&&!numbered[2].trim()&&!/[.．]/.test(numbered[1]))return true;
  const fields=(numbered?numbered[2]:text).trim().split(/[\s，,]+/),time=/^(?:日|夜|晨|午|黄昏)$/;
  const pair=(a,b)=>(time.test(a)&&/^(?:内|外)$/.test(b))||(/^(?:内|外)$/.test(a)&&time.test(b));
  return numbered?fields.some((f,i)=>i+1<fields.length&&pair(f,fields[i+1])):fields.length>=3&&pair(fields[0],fields[1]);
}
function productionNote(text){
  const line=text.trim();
  if(!/^【(?:(?:音效|字幕|画面|镜头(?:切换|切)?|转场)[：:]|特写】)/.test(line))return false;
  let depth=0;
  for(let i=0;i<line.length;i++){
    if(line[i]==='【')depth++;else if(line[i]==='】')depth--;
    if(depth===0)return i===line.length-1;
  }
  return false;
}
function spokenText(text){
  // Only the explicit visual-annotation syntax is removed from speech, never from source rows.
  let cursor=0,plain='',hasNotes=false;
  while(true){
    const match=/【搭配画面[：:]/.exec(text.slice(cursor));
    if(!match)break;
    const start=cursor+match.index;let end=start+match[0].length,depth=1;
    for(;end<text.length&&depth;end++){if(text[end]==='【')depth++;else if(text[end]==='】')depth--;}
    if(depth)return {text,hasNotes:true,valid:false};
    plain+=text.slice(cursor,start);cursor=end;hasNotes=true;
  }
  return {text:plain+text.slice(cursor),hasNotes,valid:true};
}
function dialogueFromRow(row,index,legacy=false){
  if(metadata(row.text,legacy))return null;
  const spoken=legacy?{text:row.text,valid:true}:spokenText(row.text);
  if(!spoken.valid||!legacy&&(/^\s*▲/.test(spoken.text)||metadata(spoken.text)))return null;
  const m=spoken.text.match(/^\s*([^：:△\s][^：:]{0,28})[：:]\s*(.*)$/);
  if(!m||!legacy&&/[【】]/.test(m[1]))return null;
  let speaker=m[1].trim(),rest=m[2],cue='';
  const pre=speaker.match(/^(.*?)[（(]([^）)]+)[）)]$/);
  if(pre){speaker=pre[1].trim();cue=pre[2];}
  const suffix=speaker.match(/^(.*?)(VO|OS)$/i);
  if(suffix){speaker=suffix[1].trim();cue+=(cue?'；':'')+suffix[2].toUpperCase();}
  if(!speaker||/\d+[—–-]\d+/.test(speaker))return null;
  while(/^[（(]/.test(rest)){
    const close=rest[0]==='（'?'）':')',end=rest.indexOf(close);
    if(end<0)break;
    cue+=(cue?'；':'')+rest.slice(1,end);
    rest=rest.slice(end+1).trimStart();
  }
  if(!rest.trim())return null;
  return {id:`${row.id}:D${index}`,line:row.line,speaker,quote:rest,cue,kind:kindOf({cue,quote:rest}),channel:channelOf({speaker,cue}),rowId:row.id};
}
export function normalizeSource(raw){
  if(!raw||typeof raw!=='object'||!present(raw.key))fail('SOURCE_KEY','源缺少key');
  if(raw.sourceProtocol!==undefined&&raw.sourceProtocol!==STANDARD_EPISODE_PROTOCOL)fail('SOURCE_PROTOCOL','未知正文接收协议');
  const standard=raw.sourceProtocol===STANDARD_EPISODE_PROTOCOL;
  if(standard)verifyStandardSource(raw);
  const extract=(row,index,legacy=false)=>standard?standardDialogue(row,index):dialogueFromRow(row,index,legacy);
  const key=String(raw.key),sourceRows=normalizeRows(key,raw.sourceRows),contextRows=normalizeRows(key,raw.contextRows);
  checkAliases([...sourceRows,...contextRows],key);
  const classified=sourceRows.map(row=>({...row,kind:standard?standardRowKind(row):/^\s*$/.test(row.text)?'blank':metadata(row.text)?'metadata':dialogueFromRow(row,0)||/^[！!？?，,。.……]+$/.test(row.text.trim())?'dialogue':'action'}));
  const source={...raw,key,title:raw.title??'',context:raw.context??raw.episodeContext??'',sourceRows:classified,contextRows,assets:raw.assets??[]};
  const diagnostics=[...(raw.diagnostics||[])];
  let dialogue=raw.dialogue;
  if(standard){
    const captions=classified.flatMap(row=>standardOnscreenTexts(row));
    if(raw.onscreenTexts!==undefined){
      if(!Array.isArray(raw.onscreenTexts)||raw.onscreenTexts.length!==captions.length)fail('STANDARD_ONSCREEN_COVERAGE','画面文字目录须完整且每项仅一次');
      for(let i=0;i<captions.length;i++){
        const actual=raw.onscreenTexts[i],expected=captions[i];
        if(!actual||Object.keys(actual).some(key=>!Object.hasOwn(expected,key))||['id','rowId','line','kind','text'].some(key=>actual[key]!==expected[key])||JSON.stringify(actual.parts)!==JSON.stringify(expected.parts)||JSON.stringify(actual.sourceIds)!==JSON.stringify(expected.sourceIds))fail('STANDARD_ONSCREEN_BINDING','画面文字、类型或来源坐标与原文绑定不一致');
      }
    }
    if(captions.length||['DIRECTOR_STANDARD_SCRIPT_SLICE_V3','DIRECTOR_STANDARD_SCRIPT_SLICE_V4','DIRECTOR_STANDARD_SCRIPT_SLICE_V5'].includes(raw.standardPackage?.schemaVersion))source.onscreenTexts=captions;
    const expected=classified.flatMap(row=>standardDialogues(row));
    if(dialogue!==undefined){
      if(!Array.isArray(dialogue)||dialogue.length!==expected.length)fail('STANDARD_DIALOGUE_COVERAGE','标准对白目录须完整且每段只出现一次');
      for(let i=0;i<expected.length;i++){
        const actual=dialogue[i],value=expected[i];
        if(!actual||['id','rowId','speaker','quote','cue','channel'].some(key=>actual[key]!==value[key])||JSON.stringify(actual.performanceCues)!==JSON.stringify(value.performanceCues)||actual.kind&&actual.kind!==kindOf(value)
          ||!Array.isArray(actual.sourceIds)||actual.sourceIds.length!==1||actual.sourceIds[0]!==value.rowId)fail('STANDARD_DIALOGUE_BINDING','标准对白的说者、声道、提示或原句与原文绑定不一致');
      }
    }
    dialogue=expected;
  }
  if(!dialogue){
    dialogue=[];
    for(const row of classified){
      const d=extract(row,dialogue.length+1);
      if(d)dialogue.push(d);
      else if(!standard&&row.kind==='action'&&!/^\s*[△▲]/.test(row.text)&&!productionNote(row.text)){
        const spoken=spokenText(row.text),previous=dialogue.at(-1);
        const prior=previous?classified.findIndex(r=>r.id===(previous.sourceIds?.at(-1)||previous.rowId)):-1,current=classified.indexOf(row);
        const head=previous&&classified.find(r=>r.id===previous.rowId);
        const cueOnly=/^\s*[^：:]+[：:]\s*[（(][^）)]+[）)]\s*$/.test(spoken.text);
        const emptyActor=spoken.hasNotes&&/^\s*[^：:△▲\s][^：:]{0,28}[：:]\s*$/.test(spoken.text);
        const boundary=/^\s*[△▲]/.test(spoken.text)||metadata(spoken.text)||cueOnly||emptyActor;
        const continuation=spoken.valid&&!boundary&&!/^\s*[【】]/.test(spoken.text)&&spoken.hasNotes&&spoken.text.trim()&&previous?.channel==='VO'&&head&&spokenText(head.text).hasNotes&&prior>=0&&classified.slice(prior+1,current).every(r=>r.kind==='blank');
        if(continuation){previous.quote+=spoken.text;previous.sourceIds=[...(previous.sourceIds||[previous.rowId]),row.id];}
        else if(!(spoken.valid&&(boundary||spoken.hasNotes&&!spoken.text.trim())))diagnostics.push({rowId:row.id,reason:'无法安全识别为对白'});
      }
      if(!d&&/^[！!？?，,。.……]+$/.test(row.text.trim())&&dialogue.length){
        const previous=dialogue.at(-1),prior=classified.findIndex(r=>r.id===(previous.sourceIds?.at(-1)||previous.rowId)),current=classified.indexOf(row);
        // Blank layout rows do not break a wrapped utterance; meaningful rows still do.
        if(prior>=0&&prior<current&&classified.slice(prior+1,current).every(r=>r.kind==='blank')){previous.quote+=row.text;previous.sourceIds=[...(previous.sourceIds||[previous.rowId]),row.id];}
      }
    }
  }
  source.dialogue=dialogue.map((d,i)=>{
    const lineMatches=present(d.line)?classified.filter(r=>r.line===d.line):[];
    if(!d.sourceIds?.length&&!d.rowId&&!d.sourceId&&lineMatches.length>1)fail('DIALOGUE_SOURCE_AMBIGUOUS',`对白${d.id||i+1}源行歧义`);
    const refs=d.sourceIds?.length?d.sourceIds:[d.rowId??d.sourceId??lineMatches[0]?.id??null];
    if(!refs.length||!refs[0])fail('DIALOGUE_SOURCE',`对白${d.id||i+1}缺少源行`);
    const resolved=refs.map(ref=>resolveSourceRef(source,ref));
    if(standard&&resolved.length!==1)fail('DIALOGUE_SOURCE','标准对白已按完整节点归属，不可二次跨节点拼接');
    let extracted=standard?standardDialogues(resolved[0]).find(value=>value.id===d.id):extract(resolved[0],i+1);
    let fullQuote=extracted?.quote+resolved.slice(1).map(r=>spokenText(r.text).text).join('');
    // Saved explicit catalogs are evidence, not a request to re-infer or clean old paid dialogue.
    if(raw.dialogue&&!standard){
      const old=dialogueFromRow(resolved[0],i+1,true),oldQuote=old?.quote+resolved.slice(1).map(r=>r.text).join('');
      if(old&&oldQuote===d.quote&&old.speaker===d.speaker&&old.channel===(present(d.channel)?channelOf({channel:d.channel}):old.channel)){extracted=old;fullQuote=oldQuote;}
    }
    if(!extracted)fail('DIALOGUE_SOURCE',`对白${d.id||i+1}源行不是明确对白`);
    if(fullQuote!==d.quote)fail('DIALOGUE_QUOTE',`对白${d.id||i+1}原句不完整或不一致`);
    if(extracted.speaker!==d.speaker)fail('DIALOGUE_SPEAKER',`对白${d.id||i+1}说者与源行不一致`);
    const channel=present(d.channel)?channelOf({channel:d.channel}):extracted.channel;
    if(extracted.channel!==channel)fail('DIALOGUE_CHANNEL',`对白${d.id||i+1}声道与源行不一致`);
    if(d.kind&&d.kind!=='speech'&&kindOf({quote:d.quote,cue:d.cue,channel:d.channel})==='speech')fail('DIALOGUE_KIND',`对白${d.id||i+1}声类与有词原句不符`);
    const cue=d.cue??(d.channel==='画外咆哮'?'画外咆哮':'');
    return {...d,id:d.id||`${resolved[0].id}:D${i+1}`,cue,kind:kindOf({...d,cue}),originalChannel:d.originalChannel??(d.channel==='画外咆哮'?'画外咆哮':extracted.cue.includes('画外咆哮')?'画外咆哮':d.channel??extracted.channel),channel,rowId:resolved[0].id,sourceIds:resolved.map(r=>r.id)};
  });
  const dialogueIds=source.dialogue.map(d=>d.id);
  if(new Set(dialogueIds).size!==dialogueIds.length)fail('DIALOGUE_ID_DUPLICATE','对白ID重复');
  const ownedRows=new Set(source.dialogue.flatMap(d=>d.sourceIds));
  source.sourceRows=classified.map(row=>{
    if(ownedRows.has(row.id))return row.kind==='dialogue'?row:{...row,kind:'dialogue'};
    if(!/^[！!？?，,。.……]+$/.test(row.text.trim()))return row;
    diagnostics.push({rowId:row.id,reason:'孤立标点无法归属对白'});
    return {...row,kind:'action'};
  });
  source.entryFacts=(raw.entryFacts||[]).map((f,i)=>{
    if(!f.id||!f.subject||!f.statement||!Array.isArray(f.sourceRefs)||!f.sourceRefs.length||(!f.evidence&&f.evidenceMode!=='source_rows'))fail('ENTRY_FACT',`前情事实${f.id||i+1}缺少字段`);
    if(f.evidenceMode!==undefined&&f.evidenceMode!=='source_rows')fail('ENTRY_FACT','未知前情依据模式');
    const refs=f.sourceRefs.map(ref=>resolveSourceRef(source,ref,{allowContext:true}));
    if(refs.some(row=>row.evidenceRole==='guidance'))fail('ENTRY_FACT_GUIDANCE','辅助意见不能自证入口事实，请引用真实剧情原文');
    let evidence=f.evidence;
    const matches=parts=>Array.isArray(parts)&&parts.length===refs.length&&parts.every((part,j)=>typeof part==='string'&&part.trim()&&refs[j].text.includes(part));
    if(f.evidenceMode==='source_rows'){
      const extracted=refs.map(r=>r.text);
      if(evidence!==undefined&&JSON.stringify(evidence)!==JSON.stringify(extracted))fail('ENTRY_FACT_EVIDENCE',`前情事实${f.id}显式依据与提取原文不一致`);
      evidence=extracted;
    }else if(typeof evidence==='string'&&evidence.trim()&&refs.map(r=>r.text).join('').includes(evidence)){
      // Keep exact legacy quotations byte-for-byte, including source punctuation.
    }else if(!matches(evidence)){
      const segments=typeof evidence==='string'?[ /\s*／\s*/, /\r?\n/, /\s+\|\s+/ ].map(separator=>evidence.split(separator).map(p=>p.trim())).find(parts=>parts.length>1&&matches(parts)):null;
      if(!segments)fail('ENTRY_FACT_EVIDENCE',`前情事实${f.id}依据无法在原文定位`);
      evidence=segments;
    }
    return {...f,evidence,sourceRefs:refs.map(r=>r.id)};
  });
  if(new Set(source.entryFacts.map(f=>f.id)).size!==source.entryFacts.length)fail('ENTRY_FACT_ID_DUPLICATE','前情事实ID重复');
  source.diagnostics=diagnostics;
  return source;
}
export function parseScript(text,{key,title='',context='',contextRows=[]}={}){
  if(typeof text!=='string')fail('SCRIPT_TEXT','脚本须为文本');
  const rows=text.split(/\r\n|\n|\r/).filter((line,i,a)=>i<a.length-1||line!=='').map((line,i)=>{
    const decorated=line.match(/^\s*L(\d+)\s*\|\s*P(\d+)(?:\s*\|)?[ \t　]+(.*)$/i);
    if(decorated)return {sourceId:`P${decorated[2]}`,line:Number(decorated[2]),sourceFileLine:Number(decorated[1]),numericLineAlias:false,text:decorated[3]};
    const p=line.match(/^\s*P(\d+)(?:\s*\|)?[ \t　]+(.*)$/i);
    if(p)return {sourceId:`P${p[1]}`,line:Number(p[1]),numericLineAlias:false,text:p[2]};
    const l=line.match(/^\s*L(\d+)\s*\|\s?(.*)$/i);
    if(l)return {line:Number(l[1]),text:l[2]};
    return {line:i+1,text:line};
  });
  return normalizeSource({key,title,context,contextRows,sourceRows:rows});
}
