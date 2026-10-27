#!/usr/bin/env node
// Project-adopted visual prose. Validation proves coverage references and transport,
// not the artistic sufficiency of the prose or actual video appearance.
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {realpathSync} from 'node:fs';
import {dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {isDeepStrictEqual} from 'node:util';
import {buildVisualBasis} from './core/runtime/storyboard-v4/core/visual-style.mjs';

export const VISUAL_BASELINE_VERSION='series-visual-baseline-v1';
export const COVERAGE_KEYS=Object.freeze(['form','rendering','materials','colorLighting','aspectRatio','localExceptions']);
const text=v=>typeof v==='string'&&v.trim().length>0&&!/[\r\0]/.test(v);
const need=(ok,message)=>{if(!ok)throw Object.assign(new Error(message),{code:'VISUAL_BASELINE'});};
const object=v=>v&&typeof v==='object'&&!Array.isArray(v);
const keys=(value,required,optional=[])=>{
 need(object(value),'需要对象');
 need(required.every(k=>Object.hasOwn(value,k))&&Object.keys(value).every(k=>required.includes(k)||optional.includes(k)),'字段缺失或包含未知字段');
};

export function validateVisualBaseline(b,{allowCandidate=false}={}){
 keys(b,['version','id','revision','status','visualStyle','coverage'],['adoptionBasis']);
 need(b.version===VISUAL_BASELINE_VERSION,'未知视觉基准版本');
 need(text(b.id)&&!b.id.includes('\n')&&Number.isSafeInteger(b.revision)&&b.revision>0,'基准ID或版本无效');
 need(b.status==='adopted'||(allowCandidate&&b.status==='candidate'),'视觉基准尚未采用');
 need(text(b.visualStyle),'完整视觉基准为空');
 keys(b.coverage,COVERAGE_KEYS);
 for(const k of COVERAGE_KEYS)need(text(b.coverage[k])&&b.visualStyle.includes(b.coverage[k]),`coverage.${k} 必须指向完整视觉基准中实际存在的原文`);
 need(new Set(Object.values(b.coverage)).size>1,'各维度需要具体对应的依据，不能用同一句标签代替全部维度');
 if(b.status==='adopted')need(text(b.adoptionBasis),'采用基准需要真实采用依据');
 else if(b.adoptionBasis!==undefined)need(text(b.adoptionBasis),'采用依据无效');
 return b;
}

export function proposeVisualBaseline(definition){
 const custom=Object.hasOwn(definition??{},'visualStyle');
 keys(definition,custom?['id','visualStyle','coverage']:['id','preset','aspectRatio'],custom?['revision']:['revision','localExceptions']);
 const basis=custom?{visualStyle:definition.visualStyle,coverage:structuredClone(definition.coverage)}:
  buildVisualBasis({style:definition.preset,aspectRatio:definition.aspectRatio,localExceptions:definition.localExceptions});
 const candidate={version:VISUAL_BASELINE_VERSION,id:definition.id,revision:definition.revision??1,status:'candidate',...basis};
 validateVisualBaseline(candidate,{allowCandidate:true});return candidate;
}

export function adoptVisualBaseline(candidate,basis){
 validateVisualBaseline(candidate,{allowCandidate:true});
 need(candidate.status==='candidate','仅采用候选；修改已采用基准须创建新版本候选');
 const adopted={...structuredClone(candidate),status:'adopted',adoptionBasis:basis};
 validateVisualBaseline(adopted);return adopted;
}

export function applyVisualBaseline(bundle,baseline){
 validateVisualBaseline(baseline);
 need(bundle?.version==='series-storyboard-bundle-v1'&&Array.isArray(bundle.episodes)&&bundle.episodes.length,'需要完整便携bundle');
 const updated=structuredClone(bundle);
 for(const episode of updated.episodes){
  need(object(episode.settings),'每集需要既有settings；不补写共同声音或文字政策');
  episode.settings.visualStyle=baseline.visualStyle;
  episode.settings.visualBaseline=structuredClone(baseline);
 }
 return updated;
}

export function checkVisualBaselines(bundle){
 const episodes=bundle?.episodes;
 need(Array.isArray(episodes)&&episodes.length,'缺少视觉基准的集范围');
 const present=episodes.filter(e=>Object.hasOwn(e.settings??{},'visualBaseline'));
 if(!present.length)return {mode:'legacy-unverified',proof:'旧稿保真兼容；没有采用视觉基准，未验证美术充分性及跨段共同基准'};
 need(present.length===episodes.length,'不能混合有基准与无基准的集；按采用范围完成设置');
 const baseline=validateVisualBaseline(present[0].settings.visualBaseline);
 for(const e of episodes){
  validateVisualBaseline(e.settings.visualBaseline);
  need(isDeepStrictEqual(e.settings.visualBaseline,baseline),`第${e.number}集使用不同视觉基准；不能静默拼接项目风格版本`);
  need(e.settings.visualStyle===baseline.visualStyle,`第${e.number}集visualStyle与采用基准不一致，禁止各集概括或改写`);
 }
 return {mode:'adopted-baseline-v1',id:baseline.id,revision:baseline.revision,proof:'采用基准原文及覆盖依据一致；不证明美术质量、镜表无冲突或成片效果'};
}

export function composeGeneralPrompt(settings,unitDirection){
 if(Object.hasOwn(settings??{},'visualBaseline')){
  validateVisualBaseline(settings.visualBaseline);
  need(settings.visualStyle===settings.visualBaseline.visualStyle,'settings.visualStyle未完整继承采用视觉基准');
 }
 return [settings.visualStyle,settings.prohibitions,unitDirection].filter(text).join('\n');
}

async function main(){
 const [command,...args]=process.argv.slice(2),opts={};
 const allowed={propose:['--input','--output'],adopt:['--input','--basis','--output'],apply:['--baseline','--bundle','--output'],validate:['--input']}[command];
 need(allowed&&args.length%2===0,'用法：propose --input DEF --output CANDIDATE；adopt --input CANDIDATE --basis 依据 --output BASELINE；apply --baseline BASELINE --bundle BUNDLE --output NEW；validate --input BASELINE');
 for(let i=0;i<args.length;i+=2){need(allowed.includes(args[i])&&!Object.hasOwn(opts,args[i])&&text(args[i+1]),'未知、重复或缺值参数');opts[args[i]]=args[i+1];}
 need(allowed.every(k=>Object.hasOwn(opts,k)),'缺少必需参数');
 const read=async p=>JSON.parse(await readFile(p,'utf8'));
 let result;
 if(command==='propose')result=proposeVisualBaseline(await read(opts['--input']));
 if(command==='adopt')result=adoptVisualBaseline(await read(opts['--input']),opts['--basis']);
 if(command==='apply')result=applyVisualBaseline(await read(opts['--bundle']),await read(opts['--baseline']));
 if(command==='validate'){const b=validateVisualBaseline(await read(opts['--input']));console.log(JSON.stringify({ok:true,id:b.id,revision:b.revision,proof:'结构与原文覆盖引用有效；采用依据和美术充分性仍须实际阅读核定'}));return;}
 const output=resolve(opts['--output']);
 for(const key of ['--input','--bundle','--baseline'])if(opts[key])need(output!==resolve(opts[key]),'输出不可覆盖输入');
 await mkdir(dirname(output),{recursive:true});await writeFile(output,JSON.stringify(result,null,2)+'\n',{flag:'wx'});
 console.log(JSON.stringify({ok:true,output,status:result.status??'applied',proof:'本地文字处理；未调用模型或生成媒体'}));
}
if(process.argv[1]&&realpathSync(fileURLToPath(import.meta.url))===realpathSync(process.argv[1]))main().catch(e=>{console.error(`${e.code??'ERROR'}: ${e.message}`);process.exitCode=1;});
