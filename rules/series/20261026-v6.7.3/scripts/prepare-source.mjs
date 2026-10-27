#!/usr/bin/env node
/** Source preparation only: deterministic source typing is not semantic acceptance. */
import {readFile,writeFile,realpath} from 'node:fs/promises';
import {resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {prepareStandardEpisode,deriveStandardScriptLines} from './intake/standard-script-plan.mjs';
import {compileStandardScript} from './intake/standard-script.mjs';
import {standardPackageSource} from './core/runtime/storyboard-v4/core/standard-source.mjs';
import {normalizeSource} from './core/runtime/storyboard-v4/core/source.mjs';

const options={syntax:'current',postColonCue:true,inlinePerformance:true,inlinePerformanceVersion:'inline-performance-v2'};
function reject(code,message,diagnostics=[]){throw Object.assign(new Error(message),{code,diagnostics});}

// Classify explicit layout variants through the original plan/compiler; source bytes stay untouched.
function mapHeadings(prepared){
 const mappings=[],units=prepared.index.units,byId=new Map(units.map((unit,index)=>[unit.id,{...unit,line:index+1}]));
 for(const unit of byId.values()){
  const value=unit.text.replace(/(?:\r\n|\r|\n)$/,'').replace(/^[\t \uFEFF\u00a0\u3000]*/u,'').replace(/^#{1,6}[\t ]+/u,'');
  const kind=/^第[〇零一二三四五六七八九十百千万两\d]+集(?:（[^()（）\r\n]+）|\([^()（）\r\n]+\))?(?:[ \t:：]|$)/u.test(value)?'EPISODE_HEADING':
   /^\d+[ \t]*-[ \t]*\d+(?:[ \t:：]|$)/u.test(value)||/^\d+[ \t]*\.[ \t]*\d+[\t \u00a0]*(?:日|夜|晨|昏|清晨|傍晚|凌晨|上午|下午|中午|深夜)[\t \u00a0,，、]+(?:内外|内|外)[\t \u00a0,，、]+\S/u.test(value)||/^(?:场景|SCENE)[\t :：]+[^\r\n:：]+$/iu.test(value)?'SCENE_HEADING':null;
  if(!kind)continue;
  const entry=prepared.plan.entries.find(item=>item.unitIds.includes(unit.id));
  if(entry&&(entry.target!=='BODY'||entry.kind!=='ACTION'||entry.unitIds.length!==1))continue;
  if(!entry&&!prepared.diagnostics.some(d=>d.line===unit.line&&['UNKNOWN_COLON_LINE','AMBIGUOUS_CONTINUATION'].includes(d.code)))continue;
  if(entry){entry.target='STRUCTURE';entry.kind=kind;}
  else prepared.plan.entries.push({id:`line-${unit.line}`,unitIds:[unit.id],target:'STRUCTURE',kind,
   parts:[{kind:'SOURCE_SLICE',unitId:unit.id,start:unit.start,end:unit.end}],layout:[]});
  mappings.push({line:unit.line,unitId:unit.id,kind,basis:'explicit-heading-layout-v1'});
 }
 if(mappings.length){
  const lines=new Set(mappings.map(item=>item.line));
  prepared.diagnostics=prepared.diagnostics.filter(d=>!lines.has(d.line)||!['UNKNOWN_COLON_LINE','AMBIGUOUS_CONTINUATION'].includes(d.code));
  prepared.plan.entries.sort((a,b)=>byId.get(a.unitIds[0]).start-byId.get(b.unitIds[0]).start);
  if(!prepared.diagnostics.length)prepared.compiled=compileStandardScript(prepared.index,prepared.plan);
 }
 const headings=prepared.plan.entries.filter(entry=>entry.target==='STRUCTURE'&&entry.kind==='EPISODE_HEADING');
 if(headings.length>1)reject('INTAKE_MULTIPLE_EPISODES','单集入口检测到多个集标题；按源分集，重复或同号标题先核对归属',headings.slice(1).map(entry=>({line:byId.get(entry.unitIds[0]).line,code:'INTAKE_MULTIPLE_EPISODES',message:'保留原文；不能把多集全文绑定到一个单集 key。'})));
 return mappings;
}

export function prepareSource(text,key,knownSpeakers){
 if(typeof text!=='string'||typeof key!=='string'||!key.trim()||/[\r\n\0]/.test(key))reject('INTAKE_INPUT','text/key 输入无效');
 if(!Array.isArray(knownSpeakers)||knownSpeakers.some(name=>typeof name!=='string'||!name.trim()||/[\r\n\0]/.test(name)))reject('KNOWN_SPEAKERS','known-speakers 须是明确说者的非空字符串数组，可用 []，不可将说明标头编成角色');
 const prepared=prepareStandardEpisode(text,key,{knownSpeakers,...options});
 const structureMappings=mapHeadings(prepared);
 if(prepared.diagnostics.length)reject('INTAKE_DIAGNOSTICS','来源分型存在未解决歧义，未写 source',prepared.diagnostics);
 if(!prepared.compiled?.package)reject('INTAKE_NOT_COMPILED','没有完整编译包，未写 source');
 if(!prepared.compiled.package.script.entries.some(entry=>entry.target==='BODY'))reject('INTAKE_NO_BODY','本集只有结构或说明，没有可制作正文',[{line:1,code:'INTAKE_NO_BODY',message:'原文保留；不能将说明编成剧情或对白。'}]);
 const source=normalizeSource(standardPackageSource(prepared.compiled.package,{key,sourceText:text}));
 if(source.diagnostics?.length)reject('NORMALIZED_DIAGNOSTICS','标准 source 仍有未决歧义，未写 source',source.diagnostics);
 return {...source,intakeEvidence:{version:'series-intake-evidence-v1',sourceText:text,
   sourceSha256:createHash('sha256').update(text,'utf8').digest('hex'),sourceVersion:prepared.index.sourceVersion,
   options:{knownSpeakers,...options},structureMappings,plan:prepared.plan,lines:deriveStandardScriptLines(prepared),receipt:prepared.compiled.receipt,
   proof:'机械回执仅证明原文/UTF16坐标、完整归属与分型映射；不证明语义正确、内容审核或用户验收。',semantic:'NOT_RUN'}};
}

async function main(){
 const args=process.argv.slice(2),params={};
 if(args.length%2)reject('USAGE','参数须成对：--text FILE --key E01 --known-speakers FILE --output FILE');
 for(let i=0;i<args.length;i+=2){if(!['--text','--key','--known-speakers','--output'].includes(args[i])||Object.hasOwn(params,args[i])||!args[i+1])reject('USAGE','参数未知、重复或为空');params[args[i]]=args[i+1];}
 if(!['--text','--key','--known-speakers','--output'].every(key=>params[key]))reject('USAGE','缺少 --text/--key/--known-speakers/--output');
 const decode=data=>new TextDecoder('utf-8',{fatal:true,ignoreBOM:true}).decode(data);
 const text=decode(await readFile(params['--text']));
 const names=JSON.parse(decode(await readFile(params['--known-speakers'])).replace(/^\uFEFF/,''));
 const source=prepareSource(text,params['--key'],names);
 await writeFile(params['--output'],JSON.stringify(source,null,2)+'\n',{flag:'wx'});
 console.log(JSON.stringify({ok:true,output:resolve(params['--output']),sourceProtocol:source.sourceProtocol,
  sourceVersion:source.intakeEvidence.sourceVersion,sourceSha256:source.intakeEvidence.sourceSha256,
  packageVersion:source.standardPackage.schemaVersion,physicalLines:source.intakeEvidence.lines.length,
  bodyNodes:source.sourceRows.filter(row=>row.standard.target==='BODY').length,dialogue:source.dialogue.length,
  onscreenTexts:source.onscreenTexts?.length||0,diagnostics:[],semantic:'NOT_RUN',proof:source.intakeEvidence.proof}));
}

// Node resolves symlink module filenames; compare real paths so /tmp links still run the CLI.
if(process.argv[1]&&await realpath(resolve(process.argv[1])).catch(()=>null)===await realpath(fileURLToPath(import.meta.url)))main().catch(error=>{
 const status={ok:false,code:error.code||'INTAKE_ERROR',message:error.message,diagnostics:error.diagnostics||[]};
 console.log(JSON.stringify(status));console.error(`${status.code}: ${status.message}`);process.exitCode=1;
});
