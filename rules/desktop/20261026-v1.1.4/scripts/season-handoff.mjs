#!/usr/bin/env node
import {readFile,writeFile,realpath,stat,mkdir} from 'node:fs/promises';
import {realpathSync} from 'node:fs';
import {resolve,relative,isAbsolute,dirname,sep} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {isDeepStrictEqual} from 'node:util';
import {validateSeasonScope,hashSeasonScope,checkSeasonBundle} from './season-scope.mjs';
import {checkBundle,verifyMarkdown} from './storyboard.mjs';

const text=v=>typeof v==='string'&&v.trim()&&!/[\0\r\n]/.test(v);
const object=v=>v&&typeof v==='object'&&!Array.isArray(v);
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const need=(ok,code,message)=>{if(!ok)throw Object.assign(new Error(message),{code});};
const record=(v,label)=>need(object(v)&&['user','delegated'].includes(v.kind)&&text(v.basis),'HANDOFF_ADOPTION',`${label}须记录user/delegated及实际采用依据；校验器不会替人采用`);
const fields=(v,keys,code,label)=>need(object(v)&&Object.keys(v).every(k=>keys.includes(k)),code,`${label}含未知字段或类型错误`);
const json=(bytes,label)=>{try{return JSON.parse(bytes.toString('utf8'));}catch{throw Object.assign(new Error(`${label}不是有效JSON`),{code:'HANDOFF_JSON'});}};

async function fileRef(ref,root,verify){
 fields(ref,['path','sha256'],'HANDOFF_PATH','交接文件引用');
 need(text(ref.path)&&!isAbsolute(ref.path)&&!ref.path.includes('\\')&&!ref.path.split('/').some(p=>!p||p==='.'||p==='..')&&!ref.path.includes(':'),'HANDOFF_PATH','交接路径须为包内相对路径，不得上跳或使用绝对路径');
 let file;
 try{file=await realpath(resolve(root,ref.path));}catch{throw Object.assign(new Error(`交接文件不存在：${ref.path}`),{code:'HANDOFF_PATH'});}
 const rel=relative(root,file);
 need(rel&&!rel.startsWith(`..${sep}`)&&rel!=='..'&&!isAbsolute(rel)&&(await stat(file)).isFile(),'HANDOFF_PATH','交接文件越出对接目录或不是普通文件');
 const bytes=await readFile(file),digest=sha(bytes);
 need(!verify||/^[a-f0-9]{64}$/.test(ref.sha256),'HANDOFF_FILE_HASH',`${ref.path}缺少冻结文件hash`);
 need(ref.sha256===undefined||ref.sha256===digest,'HANDOFF_FILE_HASH',`${ref.path}已变化，须重新核源收口`);
 return {ref:{path:ref.path,sha256:digest},bytes};
}

function actualTail(bundle){
 const episode=bundle.episodes.at(-1),unit=episode?.draft?.units.at(-1),shot=unit?.shots.at(-1);
 need(unit&&shot,'HANDOFF_TAIL','闭季缺少实际末单元/末镜');
 return {episode:episode.number,sceneId:unit.sceneId,unitId:unit.id,shotId:shot.id,exitState:unit.exitState,lastPicture:shot.picture,lastSound:shot.sound};
}
function continuityCheck(value,scope,tail,bundle,registry){
 need(value?.version==='series-season-continuity-v1'&&value.scopeHash===hashSeasonScope(scope),'HANDOFF_CONTINUITY','尾态文档须绑定当前季范围');
 need(isDeepStrictEqual(value.tail,{episode:tail.episode,sceneId:tail.sceneId,unitId:tail.unitId,shotId:tail.shotId}),'HANDOFF_TAIL','尾态文档坐标与实际采用末镜不一致');
 record(value.review,'尾态核源审阅');
 need(Array.isArray(value.unresolved)&&value.unresolved.length===0,'HANDOFF_CONTINUITY','仍有未决承接冲突，不能签为闭季');
 need(Array.isArray(value.deferred)&&value.deferred.every(text)&&Array.isArray(value.states)&&Array.isArray(value.openHooks)&&value.openHooks.length,'HANDOFF_CONTINUITY','须明确状态、原生未兑现钩子和延后事项');
 const refs=new Set(bundle.episodes.flatMap(e=>e.source.sourceRows.map(r=>r.id)));
 const sourceRefs=v=>Array.isArray(v)&&v.length&&new Set(v).size===v.length&&v.every(r=>text(r)&&refs.has(r));
 const assets=new Map(registry.assets.map(a=>[a.id,a])),seen=new Set();
 for(const state of value.states){
  const asset=assets.get(state?.assetId);
  need(asset?.type==='CH'&&asset.revision===state.assetRevision&&!seen.has(asset.id)&&text(state.position)&&text(state.emotion)&&sourceRefs(state.sourceRefs),'HANDOFF_CONTINUITY','人物尾态须绑定已知资产版本、位置/情绪与实际源锚，且不重复');seen.add(asset.id);
  need(['knowledge','heldProps','injuries'].every(k=>Array.isArray(state[k])&&state[k].every(text))&&state.heldProps.every(id=>assets.get(id)?.type==='PR'),'HANDOFF_CONTINUITY','人物知识/持物/伤势须逐项写清；持物引用稳定PR资产ID');
 }
 need(value.states.length||text(value.stateBasis),'HANDOFF_CONTINUITY','无人物尾态时须解释实际末场及前场状态位置');
 for(const hook of value.openHooks)need(text(hook?.text)&&sourceRefs(hook.sourceRefs),'HANDOFF_CONTINUITY','未兑现钩子须有实际源锚，不按关键词编造');
 return value;
}

async function inspect(input,{root,verify=false}={}){
 need(root,'HANDOFF_PATH','须指定对接包根目录');root=await realpath(resolve(root));
 fields(input,['version','kind','source','scope','closed','scopeHash','resume','actualTail','reusableAssets'],'HANDOFF_SCHEMA','交接包');
 need(input.version==='series-season-handoff-v1'&&['planned','closed'].includes(input.kind),'HANDOFF_SCHEMA','未知交接包版本或种类');
 fields(input.source,['original','body','index'],'HANDOFF_SOURCE','源文件组');
 const original=await fileRef(input.source.original,root,verify),body=await fileRef(input.source.body,root,verify),index=await fileRef(input.source.index,root,verify),scopeFile=await fileRef(input.scope,root,verify);
 const scope=json(scopeFile.bytes,'季范围'),sourceIndex=json(index.bytes,'全剧源索引');
 need(object(sourceIndex)&&!Object.hasOwn(sourceIndex,'readingScope')&&!Object.hasOwn(sourceIndex,'scopeHash'),'HANDOFF_FULL_INDEX','对接包须引用完整原始索引，不能使用带readingScope/scopeHash的季范围投影索引');
 validateSeasonScope(scope,{sourceHash:original.ref.sha256,index:sourceIndex});
 need(isDeepStrictEqual(sourceIndex.episodes.map(e=>({number:e.number,sceneIds:e.scenes.map(s=>s.id)})),scope.readingScope),'HANDOFF_FULL_INDEX','对接原始索引须保留全剧readingScope的全部原集场及源序');
 const scopeHash=hashSeasonScope(scope),last=scope.episodes.at(-1),cursor=scope.readingScope.findIndex(e=>e.number===last.number),nextEpisode=scope.readingScope[cursor+1]?.number??null;
 const result={version:input.version,kind:input.kind,source:{original:original.ref,body:body.ref,index:index.ref},scope:scopeFile.ref,scopeHash,resume:{previousSeasonId:scope.id,previousScopeHash:scopeHash,nextEpisode}};
 const report={ok:true,kind:input.kind,scopeHash,readyForContinuation:false,resume:result.resume,actualTail:null,coverage:null,duration:null,reusableAssets:[],proof:'仅核引用文件、结构、范围与已声明采用依据；不证明来源语义、内容质量、用户真实采用、图片质量或成片效果'};
 if(input.kind==='planned')need(input.closed===undefined&&input.actualTail===undefined&&input.reusableAssets===undefined,'HANDOFF_PLANNED','计划包不能携带或冒充实际闭季尾态');
 else{
  fields(input.closed,['bundle','sourceLock','registry','adoptedStoryboard','continuity','adoption','images'],'HANDOFF_CLOSED','实际收口包');
  record(input.closed.adoption,'实际分镜采用');
  const bundleFile=await fileRef(input.closed.bundle,root,verify),lockFile=await fileRef(input.closed.sourceLock,root,verify),registryFile=await fileRef(input.closed.registry,root,verify),adopted=await fileRef(input.closed.adoptedStoryboard,root,verify),continuityFile=await fileRef(input.closed.continuity,root,verify);
  const bundle=json(bundleFile.bytes,'采用分镜bundle'),sourceLock=json(lockFile.bytes,'冻结原文'),registry=json(registryFile.bytes,'采用总库');
  need(bundle.testFixture===false,'HANDOFF_CLOSED','合成样例不能签为真实季的闭季包');
  const season=checkSeasonBundle(bundle,{scope,requireComplete:true});
  checkBundle(bundle,{registry,sourceLock,scope});
  verifyMarkdown(bundle,adopted.bytes.toString('utf8'),{registry,sourceLock,scope});
  const tail=actualTail(bundle);continuityCheck(json(continuityFile.bytes,'实际尾态'),scope,tail,bundle,registry);
  const projection=new Map(bundle.assets.map(a=>[a.key,a.assetId]));
  const used=new Set(bundle.episodes.flatMap(e=>e.draft.units.flatMap(u=>u.bindings.map(key=>projection.get(key)))));
  const reusableAssets=registry.assets.filter(a=>used.has(a.id)).map(a=>({assetId:a.id,assetRevision:a.revision,key:a.key,designState:a.design.state}));
  need(Array.isArray(input.closed.images),'HANDOFF_IMAGE','须明确图像采用记录；尚未生图可为空数组');
  const images=[],imageKeys=new Set();
  for(const entry of input.closed.images){
   fields(entry,['assetId','assetRevision','state','file','adoption'],'HANDOFF_IMAGE','图像记录');
   need(reusableAssets.some(a=>a.assetId===entry.assetId&&a.assetRevision===entry.assetRevision)&&['candidate_pending_user','adopted'].includes(entry.state),'HANDOFF_IMAGE','图像须绑定本季实际资产版本并区分候选/采用');
   need(!imageKeys.has(entry.assetId),'HANDOFF_IMAGE','同一稳定资产版本的图像记录重复');imageKeys.add(entry.assetId);
   if(entry.state==='adopted')need(object(entry.adoption)&&['user','delegated'].includes(entry.adoption.kind)&&text(entry.adoption.basis),'HANDOFF_IMAGE_ADOPTION','图像不能从候选自动变为采用，须记录明确采用依据');
   else need(entry.adoption===undefined,'HANDOFF_IMAGE_ADOPTION','候选图不能混入已采用依据');
   const file=await fileRef(entry.file,root,verify);images.push({...entry,file:file.ref});
  }
  result.closed={bundle:bundleFile.ref,sourceLock:lockFile.ref,registry:registryFile.ref,adoptedStoryboard:adopted.ref,continuity:continuityFile.ref,adoption:structuredClone(input.closed.adoption),images};
  result.actualTail=tail;result.reusableAssets=reusableAssets;
  Object.assign(report,{readyForContinuation:true,actualTail:tail,coverage:season.coverage,duration:season.duration,reusableAssets});
 }
 for(const key of ['scopeHash','resume','actualTail','reusableAssets'])if(verify||input[key]!==undefined)need(isDeepStrictEqual(input[key],result[key]),'HANDOFF_BINDING',`${key}与实际文件或季游标不一致`);
 return {handoff:result,report};
}
export async function buildSeasonHandoff(input,options={}){return (await inspect(input,options)).handoff;}
export async function verifySeasonHandoff(input,options={}){return (await inspect(input,{...options,verify:true})).report;}
export async function verifyContinuationScope(handoff,nextScope,options={}){
 const report=await verifySeasonHandoff(handoff,options);
 need(report.readyForContinuation,'HANDOFF_NOT_CLOSED','计划包不能作为续季实际承接依据；须先完成上季收口');
 need(report.resume.nextEpisode!==null,'HANDOFF_SOURCE_ENDED','旧源已到末尾；须先独立接入追加剧本，不能伪造下一原集');
 const root=await realpath(resolve(options.root));
 const previousScope=json((await fileRef(handoff.scope,root,true)).bytes,'上季范围'),index=json((await fileRef(handoff.source.index,root,true)).bytes,'全剧源索引');
 need(nextScope?.sourceHash===previousScope.sourceHash,'HANDOFF_SOURCE_TRANSITION','追加或修改的新源暂不支持自动跨源迁移；先完成保真接入与资产继承核对');
 const sourceView={version:index.version,sourceHash:index.sourceHash,episodes:index.episodes};
 validateSeasonScope(nextScope,{sourceHash:previousScope.sourceHash,index:sourceView});
 need(nextScope.id!==previousScope.id&&nextScope.seasonNumber===previousScope.seasonNumber+1&&isDeepStrictEqual(nextScope.continuation,report.resume),'HANDOFF_CONTINUATION','续季须精确继承上季hash、季ID和下一未制作原集，不得重做上季');
 return {ok:true,previousScopeHash:report.scopeHash,nextScopeHash:hashSeasonScope(nextScope),nextEpisode:report.resume.nextEpisode,proof:'同源的闭季文件与续季游标一致；未启动续季生产，不证明内容承接质量'};
}

async function main(){
 const [command,...args]=process.argv.slice(2),options={};
 need(['create','verify','verify-next'].includes(command)&&args.length%2===0,'USAGE','season-handoff.mjs create|verify|verify-next --input REQUEST.json --root PACKAGE [--output HANDOFF.json] [--next-scope S2.json]');
 for(let i=0;i<args.length;i+=2){need(['--input','--root','--output','--next-scope'].includes(args[i])&&!Object.hasOwn(options,args[i])&&text(args[i+1]),'USAGE','参数未知、重复或缺值');options[args[i]]=args[i+1];}
 need(options['--input']&&options['--root']&&(command==='create'?options['--output']:!options['--output'])&&(command==='verify-next'?options['--next-scope']:!options['--next-scope']),'USAGE','create须指定output，verify只读；verify-next须指定next-scope');
 const input=json(await readFile(options['--input']),'交接请求'),context={root:options['--root']};
 if(command==='verify-next'){console.log(JSON.stringify(await verifyContinuationScope(input,json(await readFile(options['--next-scope']),'续季范围'),context)));return;}
 if(command==='verify'){console.log(JSON.stringify(await verifySeasonHandoff(input,context)));return;}
 const handoff=await buildSeasonHandoff(input,context),file=resolve(options['--output']);
 await mkdir(dirname(file),{recursive:true});await writeFile(file,JSON.stringify(handoff,null,2)+'\n',{flag:'wx'});
 console.log(JSON.stringify({ok:true,file,kind:handoff.kind,scopeHash:handoff.scopeHash,resume:handoff.resume}));
}
if(process.argv[1]&&realpathSync(fileURLToPath(import.meta.url))===realpathSync(process.argv[1]))main().catch(error=>{console.error(`${error.code||'ERROR'}: ${error.message}`);process.exitCode=1;});
