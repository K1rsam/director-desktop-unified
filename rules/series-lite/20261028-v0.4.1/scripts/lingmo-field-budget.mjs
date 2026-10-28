#!/usr/bin/env node
import {readFileSync, realpathSync} from 'node:fs';
import {fileURLToPath} from 'node:url';

// Reference envelope from one successful sample; warning only, NOT a DB schema.
export const PROP_BUDGET=Object.freeze({characters:81,utf8Bytes:243,jsonArrayCharacters:101});
export function splitNames(raw){
 if(raw==='无'||raw==='待确认')return [];
 const names=[];let name='';
 for(let i=0;i<raw.length;i++){
  const c=raw[i];
  if(c==='\\'&&(raw[i+1]==='\\'||raw[i+1]==='、'))name+=raw[++i];
  else if(c==='、'){names.push(name.trim());name='';}else name+=c;
 }
 names.push(name.trim());return names;
}
export function inspectFieldBudget(markdown){
 const lines=markdown.replace(/^\uFEFF/,'').split(/\r?\n/),rows=[];let active=false,episode='';
 for(let i=0;i<lines.length;i++){
  const line=lines[i];
  if(line==='**【标准化剧本】**'){active=true;continue;}
  if(active&&/^\*\*【.+】\*\*$/.test(line)){active=false;continue;}
  if(!active)continue;
  if(/^第[0-9零〇一二两三四五六七八九十百千万]+集$/.test(line))episode=line;
  // Only the structural scene asset triplet; do not scan prose or catalog features.
  if(!line.startsWith('场景：')||!lines[i+1]?.startsWith('人物：')||!lines[i+2]?.startsWith('道具：'))continue;
  const raw=lines[i+2].slice(3),names=splitNames(raw);
  const metrics={characters:[...raw].length,utf8Bytes:Buffer.byteLength(raw,'utf8'),jsonArrayCharacters:[...JSON.stringify(names)].length};
  rows.push({episode,scene:lines[i-1],line:i+3,count:names.length,...metrics,exceeded:Object.keys(PROP_BUDGET).filter(k=>metrics[k]>PROP_BUDGET[k])});
 }
 const failures=rows.filter(r=>r.exceeded.length);
 return {ok:rows.length>0,reviewRequired:failures.length>0,profile:'lingmo-scene-prop-observation-v2',databaseColumnLimit:'UNKNOWN',budget:PROP_BUDGET,scenes:rows.length,maxima:Object.fromEntries(Object.keys(PROP_BUDGET).map(k=>[k,Math.max(0,...rows.map(r=>r[k]))])),warnings:failures,proof:'Observed sample envelope only; exceeding it is not proof of rejection. Actual backend limit UNKNOWN.'};
}
export function assertFieldBudget(markdown){
 const report=inspectFieldBudget(markdown);
 if(!report.ok)throw Object.assign(new Error('未找到场级资产三行；不能视为容量检查通过'),{code:'LINGMO_SCENE_PROP_BUDGET',report});
 return report;
}
if(process.argv[1]&&realpathSync(process.argv[1])===realpathSync(fileURLToPath(import.meta.url))){
 try{
  if(process.argv.length!==3)throw new Error('Usage: node scripts/lingmo-field-budget.mjs FINAL.md');
  const report=inspectFieldBudget(readFileSync(process.argv[2],'utf8'));console.log(JSON.stringify(report,null,2));process.exitCode=report.ok?0:1;
 }catch(error){console.error(error.message);process.exitCode=2;}
}
