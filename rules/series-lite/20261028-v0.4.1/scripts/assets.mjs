#!/usr/bin/env node
import {readFile,writeFile,mkdir,rename,unlink,rmdir} from 'node:fs/promises';
import {realpathSync} from 'node:fs';
import {resolve,join,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash,randomUUID} from 'node:crypto';
import {validateUsageAnnotations,validateUsageEntries,assetScopeUses} from './asset-uses.mjs';
import {validateDesignLevel,validateImageDesigns,prepareImageDesignBatch,resolveImageDesign,imageHash} from './asset-image-design.mjs';
import {sanitizeImportNames,validateImportMappings,importAssetFields} from './lingmo-asset-names.mjs';
import {validateSeasonScope,hashSeasonScope,assertSeasonUse,checkSeasonBundle} from './season-scope.mjs';

const need=(ok,code,message)=>{if(!ok)throw Object.assign(new Error(message),{code});};
const text=v=>typeof v==='string'&&!!v.trim()&&!/[\0\r]/.test(v);
const name=v=>text(v)&&!/[\n、\[\]@]/.test(v)&&v!=='无';
const logicalName=v=>text(v)&&!/[\n\u2028\u2029]/.test(v)&&v!=='无';
const strings=v=>Array.isArray(v)&&v.length>0&&v.every(text)&&new Set(v).size===v.length;
const positive=v=>Number.isSafeInteger(v)&&v>0;
const digest=bytes=>createHash('sha256').update(bytes).digest('hex');
const keyName=v=>v.normalize('NFKC').trim().toLocaleLowerCase('en-US');
const snapshotName=revision=>`v${String(revision).padStart(6,'0')}.json`;
const encode=v=>JSON.stringify(v,null,2)+'\n';
const adopted=state=>['TEXT_ADOPTED','REFERENCE_ADOPTED'].includes(state);
const validUse=u=>u&&Number.isInteger(u.episode)&&text(u.sceneId)&&text(u.unitId);
const useKey=u=>JSON.stringify([u.episode,u.sceneId,u.unitId]);
const sceneUseKey=u=>JSON.stringify([u.episode,u.sceneId]);
const stable=value=>JSON.stringify(value,(_,v)=>v&&typeof v==='object'&&!Array.isArray(v)?Object.fromEntries(Object.keys(v).sort().map(k=>[k,v[k]])):v);
const objectHash=value=>digest(stable(value));
function checkScopeUses(scope,uses,label){
 need(Array.isArray(uses)&&uses.length>0&&uses.every(u=>u&&Number.isInteger(u.episode)&&text(u.sceneId))&&new Set(uses.map(sceneUseKey)).size===uses.length,'ASSET_SCOPE_USES',`${label} 须明确不重复的scopeUses集场用途，不能从名字猜测`);
 for(const use of uses)assertSeasonUse(scope,use,label);
}
function registryScope(registry,{requireLocked=false}={}){
 if(registry.productionScope===undefined){need(registry.productionScopeHash===undefined,'ASSET_SCOPE_BINDING','不能删除已绑定季范围后保留hash');return;}
 const scope=validateSeasonScope(registry.productionScope,{sourceHash:registry.sourceHash,requireLocked});
 need(registry.productionScopeHash===hashSeasonScope(scope),'ASSET_SCOPE_BINDING','总库季范围hash不匹配');return scope;
}

export function validateRegistry(registry){
 need(registry?.version==='series-assets-v1'&&text(registry.id)&&positive(registry.revision)&&/^[a-f0-9]{64}$/.test(registry.sourceHash),'ASSET_REGISTRY','总库版本、标识或源文件hash无效');
 need(Array.isArray(registry.assets)&&Array.isArray(registry.requests),'ASSET_REGISTRY','缺少资产或补项目录');
 need(registry.importProfile===undefined||registry.importProfile==='lingmo-v7','ASSET_IMPORT_PROFILE','未知总库导入协议');
 const scope=registryScope(registry);
 const v7=registry.importProfile==='lingmo-v7',assetName=v7?logicalName:name;
 if(v7)validateImportMappings(registry.assets);
 const ids=new Map(),keys=new Map(),aliases=new Map();
 for(const a of registry.assets){
  if(scope)checkScopeUses(scope,a.scopeUses,`${a?.id||'新资产'} 的范围`);
  need(a&&/^A\d{4,}$/.test(a.id)&&positive(a.revision)&&!ids.has(a.id),'ASSET_ID','资产稳定ID或版本无效、重复');
  need(['SC','CH','PR'].includes(a.type)&&assetName(a.name)&&a.key===a.name+a.type&&text(a.status)&&!keys.has(a.key),'ASSET_RECORD',`${a.id} 正式名、类型、状态或导入键无效、重复`);
  need(a.origin&&['source','design'].includes(a.origin.kind)&&strings(a.origin.sourceRefs)&&text(a.origin.basis),'ASSET_ORIGIN',`${a.id} 必须分开源漏项与导演设计，并保留源锚及依据`);
  need(a.design&&['UNADOPTED','TEXT_ADOPTED','REFERENCE_ADOPTED','STALE'].includes(a.design.state)&&(!adopted(a.design.state)||text(a.design.text)),'ASSET_DESIGN',`${a.id} 已采用资产缺少实际文字生产卡`);
  validateDesignLevel(a);
  need(Array.isArray(a.aliases)&&a.aliases.every(assetName)&&new Set(a.aliases).size===a.aliases.length,'ASSET_ALIASES',`${a.id} 别名无效`);
  if(a.originalName!==undefined)need(text(a.originalName),'ASSET_ALIASES',`${a.id} 原名无效`);
  // A source name may identify a family with several necessary states, so it is not a unique alias.
  for(const n of [a.name,...a.aliases]){
   const k=JSON.stringify([a.type,keyName(n)]),old=aliases.get(k);
   need(!old||old===a.id,'ASSET_ALIAS_CONFLICT',`${a.id} 的名称 ${n} 已归属 ${old}；核同物异名后复用ID，不能另建重复项`);aliases.set(k,a.id);
  }
  ids.set(a.id,a);keys.set(a.key,a);
 }
 for(const a of ids.values()){
  const dependencies=a.design.dependencies,seen=new Set();
  need(dependencies===undefined||Array.isArray(dependencies),'ASSET_DESIGN_DEPENDENCY',`${a.id}@${a.revision} 的design.dependencies须为数组`);
  for(const d of dependencies??[]){
   const target=ids.get(d?.assetId);
   need(d&&typeof d==='object'&&!Array.isArray(d)&&target&&positive(d.assetRevision)&&d.assetRevision<=target.revision&&text(d.purpose),'ASSET_DESIGN_DEPENDENCY',`${a.id}@${a.revision} 设计依赖须有已知assetId、有效的已存在版本assetRevision及用途purpose`);
   need(d.assetId!==a.id&&!seen.has(d.assetId),'ASSET_DESIGN_DEPENDENCY',`${a.id}@${a.revision} 的设计依赖不能指向自身或重复引用 ${d.assetId}`);seen.add(d.assetId);
  }
 }
 const active=new Set(),checked=new Set(),path=[];
 const visit=a=>{
  if(checked.has(a.id))return;
  need(!active.has(a.id),'ASSET_DESIGN_DEPENDENCY_CYCLE',`设计依赖存在循环：${[...path,a.id].join(' → ')}`);
  active.add(a.id);path.push(a.id);
  for(const d of a.design.dependencies??[])visit(ids.get(d.assetId));
  path.pop();active.delete(a.id);checked.add(a.id);
 };
 for(const a of ids.values())visit(a);
 const requests=new Set();
 for(const r of registry.requests){
  need(r&&text(r.id)&&!requests.has(r.id)&&ids.has(r.assetId)&&positive(r.assetRevision)&&r.assetRevision<=ids.get(r.assetId).revision,'ASSET_REQUEST','补项ID、目标或版本无效');requests.add(r.id);
  need(r.state===undefined||['active','withdrawn'].includes(r.state),'ASSET_REQUEST','未知补项状态');
  need(Array.isArray(r.uses)&&(r.state==='withdrawn'?r.uses.length===0:r.uses.length>0)&&r.uses.every(validUse)&&new Set(r.uses.map(useKey)).size===r.uses.length,'ASSET_REQUEST','补项须有不重复的集、场、单元坐标');
  if(scope)for(const use of r.uses)assertSeasonUse(scope,use,`${r.id} 的生产用途`);
  if(r.state==='withdrawn')need(text(r.resolution?.reason)&&text(r.resolution?.sourceReview),'ASSET_REQUEST','撤回使用请求须有来源复核与理由，不能删除源要求');
 }
 validateUsageAnnotations(registry);
 validateImageDesigns(registry);
 return {ids,keys};
}

function checkDesignDependencies(asset,ids,coordinate,path=[`${asset.id}@${asset.revision}`],checked=new Set()){
 if(checked.has(asset.id))return;
 for(const d of asset.design.dependencies??[]){
  const target=ids.get(d.assetId),chain=[...path,`${target.id}@${d.assetRevision}`];
  const context=`${coordinate} 的设计依赖 ${chain.join(' → ')}（当前 ${target.id}@${target.revision}）`;
  need(d.assetRevision===target.revision,'ASSET_DESIGN_DEPENDENCY_STALE',`${context} 版本已过期；复核实际依赖后以revise登记，不自动升级引用`);
  need(adopted(target.design.state),'ASSET_DESIGN_DEPENDENCY_UNADOPTED',`${context} 状态为 ${target.design.state}，尚无可采用的祖先设计`);
  checkDesignDependencies(target,ids,coordinate,chain,checked);
 }
 checked.add(asset.id);
}

// Checks declared consumption only. Source/design judgment and prose omissions require reading the work.
export function checkAssetBindings(bundle,registry){
 need(registry,'ASSET_REGISTRY_REQUIRED','生产稿须提供 --registry 当前统一总库；不能只核bundle内部目录');
 const {ids}=validateRegistry(registry),binding=bundle.assetRegistry;
 const scope=registryScope(registry,{requireLocked:true});
 if(scope)checkSeasonBundle(bundle,{scope});
 else need(bundle.productionScope===undefined&&bundle.productionScopeHash===undefined,'ASSET_SCOPE_BINDING','季模式生产稿须使用同季绑定总库，不能降级到旧无范围总库');
 const v7=bundle.exportProfile==='lingmo-v7';
 if(v7)need(registry.importProfile==='lingmo-v7','ASSET_IMPORT_PROFILE','v7生产稿须先为统一总库建立灵墨导入映射');
 need(bundle.sourceHash===registry.sourceHash&&binding?.id===registry.id&&positive(binding.revision)&&binding.revision<=registry.revision,'ASSET_REGISTRY_BINDING','源文件、总库身份或采用版本不匹配');
 const projection=new Map(bundle.assets.map(a=>[a.key,a])),used=new Set(),uses=[];
 for(const e of bundle.episodes)for(const u of e.draft.units)for(const key of u.bindings){
  const a=projection.get(key),current=ids.get(a?.assetId),coordinate=`第${e.number}集/${u.sceneId}/${u.id}`;
  need(current,'ASSET_REGISTRY_MISSING',`${coordinate} 的 ${key} 未回补统一总库`);
  need(a.assetRevision===current.revision,'ASSET_REGISTRY_STALE',`${coordinate} 的 ${current.id}@${a.assetRevision}/${key} 使用旧资产版本（当前 ${current.id}@${current.revision}）；仅复核实际消费它的单元`);
  for(const field of ['key','name','type','status'])need(a[field]===current[field],'ASSET_REGISTRY_MISMATCH',`${current.id} 的 ${field} 与总库不符`);
  if(v7)for(const field of ['importName','placeholderOrdinal'])need(a[field]===current[field],'ASSET_REGISTRY_MISMATCH',`${current.id} 的 ${field} 与总库不符；须更新当前资产投影`);
  need(adopted(current.design.state),'ASSET_DESIGN_UNADOPTED',`${coordinate} 的 ${current.id}@${current.revision}/${key} 状态为 ${current.design.state}，尚无可采用文字卡`);
  if(scope)assertSeasonUse(scope,{episode:e.number,sceneId:u.sceneId},`${coordinate} 的资产使用`);
  checkDesignDependencies(current,ids,coordinate);
  used.add(current.id);uses.push({episode:e.number,sceneId:u.sceneId,unitId:u.id,assetId:current.id,assetRevision:current.revision,key});
 }
 const episodes=new Set(bundle.episodes.map(e=>e.number)),closedRequests=[],pendingOutsideScope=[];
 for(const r of registry.requests){
  if(r.state==='withdrawn')continue;
  for(const target of r.uses.filter(u=>episodes.has(u.episode)))need(uses.some(u=>useKey(u)===useKey(target)&&u.assetId===r.assetId),'ASSET_REQUEST_UNCONSUMED',`${r.id} 已到料但 ${useKey(target)} 尚未实际消费 ${r.assetId}`);
  if(r.uses.every(u=>episodes.has(u.episode)))closedRequests.push(r.id);else pendingOutsideScope.push(r.id);
 }
 return {id:registry.id,currentRevision:registry.revision,bundleRevision:binding.revision,assets:[...used],uses,closedRequests,pendingOutsideScope,proof:'已声明的总库、文字卡、版本与使用坐标一致；正文漏项和来源语义仍须内容复核'};
}

export async function readRegistry(directory){
 const root=resolve(directory),head=JSON.parse(await readFile(join(root,'current.json'),'utf8'));
 need(head.version==='series-assets-head-v1'&&positive(head.revision),'ASSET_HEAD','当前总库指针无效');
 const file=join(root,snapshotName(head.revision)),bytes=await readFile(file);
 need(digest(bytes)===head.sha256,'ASSET_SNAPSHOT_CHANGED','已采用总库快照被原地修改；保留旧文件并以新版本修复');
 const registry=JSON.parse(bytes);validateRegistry(registry);
 need(registry.id===head.id&&registry.revision===head.revision,'ASSET_HEAD','总库指针与快照不符');
 return {registry,file,sha256:head.sha256};
}

async function locked(root,fn){
 await mkdir(root,{recursive:true});const lock=join(root,'.lock');
 try{await mkdir(lock);}catch(error){if(error.code==='EEXIST')throw Object.assign(new Error('总库写入锁已存在；检查仍在执行的写入者，不自动抢锁'),{code:'ASSET_LOCKED'});throw error;}
 try{return await fn();}finally{await rmdir(lock);}
}

async function publish(root,registry){
 validateRegistry(registry);const bytes=encode(registry),file=join(root,snapshotName(registry.revision)),temp=join(root,`.current-${randomUUID()}.tmp`);
 await writeFile(file,bytes,{flag:'wx'});
 try{
  await writeFile(temp,encode({version:'series-assets-head-v1',id:registry.id,revision:registry.revision,sha256:digest(bytes)}),{flag:'wx'});
  await rename(temp,join(root,'current.json'));
 }catch(error){await unlink(temp).catch(()=>{});await unlink(file).catch(()=>{});throw error;}
 return {ok:true,id:registry.id,revision:registry.revision,file,sha256:digest(bytes)};
}

export async function initializeRegistry(directory,input){
 const root=resolve(directory);return locked(root,async()=>{
  try{await readFile(join(root,'current.json'));need(false,'ASSET_EXISTS','总库已初始化，禁止重置');}catch(error){if(error.code!=='ENOENT')throw error;}
  need(Array.isArray(input.assets),'ASSET_REGISTRY','初始化须提供完整基线assets数组');
  const registry={version:'series-assets-v1',id:randomUUID(),sourceHash:input.sourceHash,revision:1,assets:input.assets.map((a,i)=>({...a,id:`A${String(i+1).padStart(4,'0')}`,revision:1})),requests:[],changes:[]};
  if(input.productionScope!==undefined){registry.productionScope=structuredClone(validateSeasonScope(input.productionScope,{sourceHash:input.sourceHash}));registry.productionScopeHash=hashSeasonScope(registry.productionScope);}
  else need(input.productionScopeHash===undefined,'ASSET_SCOPE_BINDING','季范围缺失，不能仅保留hash');
  if(input.importProfile!==undefined){
   need(input.importProfile==='lingmo-v7','ASSET_IMPORT_PROFILE','未知总库导入协议');registry.importProfile=input.importProfile;assignImportMappings(registry.assets);
  }
  return {...await publish(root,registry),assets:registry.assets.map(({id,revision,key})=>({id,revision,key}))};
 });
}

function assignImportMappings(assets,start=0){
 const values=assets.map(a=>a.importName===undefined?a.name:a.importName),sanitized=sanitizeImportNames(values);
 for(const [i,a] of assets.entries()){
  need(a.placeholderOrdinal===undefined||a.placeholderOrdinal===start+i,'ASSET_PLACEHOLDER_ORDINAL',`${a.id} 新资产占位编号必须按总库顺序追加，不可重分配`);
  a.importName=a.importName===undefined?sanitized[i]:a.importName;a.placeholderOrdinal=start+i;
 }
 validateImportMappings(assets);
}

export async function prepareImport(directory,input){
 const root=resolve(directory);return locked(root,async()=>{
  const {registry}=await readRegistry(root);
  registryScope(registry,{requireLocked:true});
  need(input?.version==='series-asset-import-v1'&&input.baseRevision===registry.revision,'ASSET_BASE_REVISION','导入映射基于旧总库，须先读取当前版本');
  need(input.importProfile==='lingmo-v7'&&registry.importProfile===undefined,'ASSET_IMPORT_PROFILE','仅可为尚无导入协议的旧库建立 lingmo-v7 映射；后续改名走 revise');
  const names=input.names??[],seen=new Set();need(Array.isArray(names),'ASSET_IMPORT_NAME','names须为按稳定ID声明的导入名数组');
  for(const n of names){
   const asset=registry.assets.find(a=>a.id===n?.id);need(asset&&!seen.has(n.id)&&typeof n.importName==='string','ASSET_IMPORT_NAME','导入名覆盖含未知或重复ID，或名称不是文字');seen.add(n.id);asset.importName=n.importName;
  }
  assignImportMappings(registry.assets);registry.importProfile='lingmo-v7';registry.revision++;
  return {...await publish(root,registry),importProfile:registry.importProfile,assetRevisionsChanged:false};
 });
}

export async function applyChange(directory,request){
 const root=resolve(directory);return locked(root,async()=>{
  const {registry}=await readRegistry(root);
  const scope=registryScope(registry,{requireLocked:true});
  need(request?.version==='series-asset-change-v1'&&request.baseRevision===registry.revision,'ASSET_BASE_REVISION','补项基于旧总库；读取当前版本并重新核重，不能覆盖并行修改');
  need(text(request.requestId)&&!registry.requests.some(r=>r.id===request.requestId)&&!(registry.changes||[]).some(r=>r.id===request.requestId)&&!(registry.usageAnnotations||[]).some(r=>r.id===request.requestId)&&!(registry.imageDesigns||[]).some(r=>r.id===request.requestId)&&text(request.reason)&&text(request.duplicateReview),'ASSET_CHANGE','缺少唯一申请ID、原因或实际核重说明');
  need(['add','revise','reuse'].includes(request.action),'ASSET_CHANGE','action须为add、revise或reuse');
  const stage=request.stage??'production';need(['asset-design','production'].includes(stage),'ASSET_CHANGE','stage须为asset-design或production');
  need(Array.isArray(request.uses)&&(stage==='asset-design'?request.uses.length===0:request.uses.length>0),'ASSET_REQUEST','阶段二尚无单元时uses为空；第三阶段必须回报实际使用坐标');
  if(scope){
   if(stage==='asset-design')checkScopeUses(scope,request.scopeUses,`${request.requestId} 的设计用途`);
   else for(const use of request.uses)assertSeasonUse(scope,use,`${request.requestId} 的生产用途`);
   if(request.asset?.scopeUses!==undefined)checkScopeUses(scope,request.asset.scopeUses,`${request.requestId} 的资产范围`);
  }
  let asset;
  if(request.action==='add'){
   const next=1+Math.max(0,...registry.assets.map(a=>Number(a.id.slice(1))));
   asset={...request.asset,id:`A${String(next).padStart(4,'0')}`,revision:1};
   if(scope)asset.scopeUses=structuredClone(stage==='asset-design'?request.scopeUses:request.uses.map(({episode,sceneId})=>({episode,sceneId})).filter((u,i,all)=>all.findIndex(v=>sceneUseKey(v)===sceneUseKey(u))===i));
   if(registry.importProfile==='lingmo-v7')assignImportMappings([asset],1+Math.max(-1,...registry.assets.map(a=>a.placeholderOrdinal)));
   registry.assets.push(asset);
  }else{
   const i=registry.assets.findIndex(a=>a.id===request.assetId);need(i>=0,'ASSET_ID','要复用/修订的稳定ID不存在');
   const old=registry.assets[i];
   if(request.action==='revise'){
    need(request.asset?.type===old.type,'ASSET_IDENTITY','修订不可借同ID改变资产类型；身份或形态变体另建且记录关系');
    if(registry.importProfile==='lingmo-v7')need(request.asset.placeholderOrdinal===undefined||request.asset.placeholderOrdinal===old.placeholderOrdinal,'ASSET_PLACEHOLDER_ORDINAL','修订必须保留稳定占位编号；新增资产才追加编号');
    asset={...old,...request.asset,id:old.id,revision:old.revision+1};registry.assets[i]=asset;
    if(scope&&stage==='asset-design')asset.scopeUses=structuredClone(request.scopeUses);
   }else{need(request.asset===undefined,'ASSET_CHANGE','reuse不能偷偷修改资产，须另用revise');asset=old;}
  }
  const change={id:request.requestId,stage,action:request.action,assetId:asset.id,assetRevision:asset.revision,reason:request.reason,duplicateReview:request.duplicateReview,uses:request.uses};
  if(scope&&stage==='asset-design')change.scopeUses=structuredClone(request.scopeUses);
  registry.changes=[...(registry.changes||[]),change];
  if(stage==='production')registry.requests.push({...change,state:'active'});
  registry.revision++;
  return {...await publish(root,registry),asset:{id:asset.id,revision:asset.revision,key:asset.key},requestId:request.requestId,state:stage==='production'?'awaiting_consumption':'recorded'};
 });
}

export async function amendRequest(directory,input){
 const root=resolve(directory);return locked(root,async()=>{
  const {registry}=await readRegistry(root);
  const scope=registryScope(registry,{requireLocked:true});
  need(input?.version==='series-asset-amend-v1'&&input.baseRevision===registry.revision,'ASSET_BASE_REVISION','使用请求修订基于旧总库');
  const request=registry.requests.find(r=>r.id===input.requestId);
  need(request&&['retarget','withdraw'].includes(input.action)&&text(input.reason)&&text(input.sourceReview),'ASSET_REQUEST','修订须有已登记请求、来源复核和理由');
  if(scope&&Array.isArray(input.uses))for(const use of input.uses)assertSeasonUse(scope,use,`${input.requestId} 的修订用途`);
  request.uses=input.uses;request.state=input.action==='withdraw'?'withdrawn':'active';
  request.resolution={action:input.action,reason:input.reason,sourceReview:input.sourceReview,revision:registry.revision+1};registry.revision++;
  return {...await publish(root,registry),requestId:request.id,state:request.state};
 });
}

// Batch additive uses: publish one registry snapshot without touching any design row.
export async function recordAssetUses(directory,input){
 const root=resolve(directory);return locked(root,async()=>{
  const loaded=await readRegistry(root),{registry}=loaded;
  registryScope(registry,{requireLocked:true});
  need(input&&Object.keys(input).every(k=>['version','baseRevision','requestId','reason','entries'].includes(k))&&input.version==='series-asset-uses-v1'&&positive(input.baseRevision)&&text(input.requestId)&&text(input.reason),'ASSET_USAGE','用途登记仅接受version/baseRevision/requestId/reason/entries，不接受设计或范围修订');
  const previous=(registry.usageAnnotations??[]).find(r=>r.id===input.requestId);
  if(previous){
   need(previous.reason===input.reason&&stable(previous.entries)===stable(input.entries),'ASSET_USAGE','用途requestId已用于不同内容');
   validateUsageEntries(input.entries,registry,{current:true});
   const {ids}=validateRegistry(registry);for(const entry of input.entries)checkDesignDependencies(ids.get(entry.assetId),ids,`用途登记 ${entry.assetId}`);
   return {ok:true,id:registry.id,revision:registry.revision,file:loaded.file,sha256:loaded.sha256,changed:false,assetRevisionsChanged:false,requestId:input.requestId};
  }
  need(input.baseRevision===registry.revision,'ASSET_BASE_REVISION','用途登记基于旧总库，读取现版后重试，不重做设计');
  need(!(registry.changes??[]).some(r=>r.id===input.requestId)&&!registry.requests.some(r=>r.id===input.requestId)&&!(registry.imageDesigns??[]).some(r=>r.id===input.requestId),'ASSET_USAGE','requestId已被其它修改使用');
  validateUsageEntries(input.entries,registry,{current:true});
  const {ids}=validateRegistry(registry);
  for(const entry of input.entries)checkDesignDependencies(ids.get(entry.assetId),ids,`用途登记 ${entry.assetId}`);
  const already=new Set((registry.usageAnnotations??[]).flatMap(r=>r.entries.flatMap(e=>e.uses.map(u=>stable([e.assetId,e.assetRevision,u])))));
  const entries=input.entries.map(e=>({...e,uses:e.uses.filter(u=>!already.has(stable([e.assetId,e.assetRevision,u])))})).filter(e=>e.uses.length);
  if(!entries.length)return {ok:true,id:registry.id,revision:registry.revision,file:loaded.file,sha256:loaded.sha256,changed:false,assetRevisionsChanged:false};
  registry.revision++;
  registry.usageAnnotations=[...(registry.usageAnnotations??[]),{id:input.requestId,reason:input.reason,recordedAtRevision:registry.revision,entries:structuredClone(input.entries)}];
  return {...await publish(root,registry),changed:true,assetRevisionsChanged:false,requestId:input.requestId,entries:entries.length};
 });
}

export async function recordImageDesign(directory,input){
 const root=resolve(directory);return locked(root,async()=>{
  const loaded=await readRegistry(root),{registry}=loaded;registryScope(registry,{requireLocked:true});
  // Validate shape and current narrative/parent dependencies even for a retry.
  const batch=prepareImageDesignBatch(registry,input),previous=(registry.imageDesigns??[]).find(r=>r.id===input.requestId);
  if(previous){
   need(previous.inputHash===batch.inputHash,'ASSET_IMAGE_DESIGN','制图requestId已用于不同内容');
   for(const entry of previous.entries){const current=resolveImageDesign(registry,entry.assetId);need(current.imageDesignRevision===entry.imageDesignRevision,'ASSET_IMAGE_STALE','该请求已有后续制图版本，不能当作当前采用重试');}
   return {ok:true,id:registry.id,revision:registry.revision,file:loaded.file,sha256:loaded.sha256,changed:false,assetRevisionsChanged:false,requestId:input.requestId};
  }
  need(input.baseRevision===registry.revision,'ASSET_BASE_REVISION','制图补充基于旧总库；读取现版重试，不重做分镜');
  need(![...registry.requests,...(registry.changes??[]),...(registry.usageAnnotations??[])].some(r=>r.id===input.requestId),'ASSET_IMAGE_DESIGN','requestId已被其它修改使用');
  registry.imageDesigns=[...(registry.imageDesigns??[]),batch];registry.revision++;
  return {...await publish(root,registry),changed:true,assetRevisionsChanged:false,requestId:input.requestId,entries:batch.entries.map(({assetId,imageDesignRevision})=>({assetId,imageDesignRevision}))};
 });
}

// Only explicit consumers become image work. A design ancestor is a read dependency, never an implicit image task.
export async function createImageQueue(directory,input){
 const loaded=await readRegistry(directory),{registry}=loaded,{ids}=validateRegistry(registry),scope=registryScope(registry,{requireLocked:true});
 need(input?.version==='series-image-request-v1'&&input.baseRevision===registry.revision,'ASSET_BASE_REVISION','图片用途清单须基于当前总库版本');
 need(Array.isArray(input.uses)&&input.uses.length>0,'ASSET_IMAGE_USES','图片清单须显式列出实际需要图像的uses，不能扫描全库');
 const items=new Map(),seen=new Set(),imageCache=new Map();let separated=false;
 for(const use of input.uses){
  const asset=ids.get(use?.assetId),coordinate=`图片用途 ${use?.assetId||'?'} / 第${use?.episode}集/${use?.sceneId}`;
  need(asset&&use.assetRevision===asset.revision,'ASSET_REGISTRY_STALE',`${coordinate} 缺资产或设计版本不是当前采用版本`);
  need(Number.isInteger(use.episode)&&text(use.sceneId)&&text(use.purpose),'ASSET_IMAGE_USES',`${coordinate} 须明确episode、sceneId、purpose`);
  const duplicate=JSON.stringify([use.assetId,use.assetRevision,use.episode,use.sceneId,use.purpose]);
  need(!seen.has(duplicate),'ASSET_IMAGE_USES',`${coordinate} 图片用途重复`);seen.add(duplicate);
  if(scope){
   assertSeasonUse(scope,use,coordinate);
   need(assetScopeUses(registry,asset).some(u=>sceneUseKey(u)===sceneUseKey(use)),'ASSET_SCOPE_USES',`${coordinate} 未在该资产的源用途scopeUses登记；先核源补回总库`);
  }
  need(asset.imageRequired!==false,'ASSET_IMAGE_NOT_REQUIRED',`${coordinate} 明确登记为不生图项，不能自动转为图片任务`);
  need(adopted(asset.design.state),'ASSET_DESIGN_UNADOPTED',`${coordinate} 尚无采用设计`);
  checkDesignDependencies(asset,ids,coordinate);
  const image=resolveImageDesign(registry,asset.id,imageCache);
  if(!items.has(asset.id)){
   const filename=registry.importProfile==='lingmo-v7'?importAssetFields(asset).filename:`${{CH:'角色',SC:'场景',PR:'道具'}[asset.type]}-${asset.name}.png`;
   need(!/[\\/\x00-\x1f:*?"<>|]/.test(filename)&&Buffer.byteLength(filename,'utf8')<=255,'ASSET_IMAGE_FILENAME',`${coordinate} 缺安全精确图片文件名；先建立导入映射`);
   const dependencies=[],visited=new Set();
   const collect=current=>{for(const d of current.design.dependencies??[]){if(visited.has(d.assetId))continue;visited.add(d.assetId);const target=ids.get(d.assetId),parent=resolveImageDesign(registry,d.assetId,imageCache);dependencies.push({...d,designHash:parent.designHash,designState:parent.design.state,...(image.imageDesignRevision?{imageDesignRevision:parent.imageDesignRevision}:{})});collect(target);}};collect(asset);
   const designHash=image.designHash,itemId=`${asset.id}@${asset.revision}`;
   const identity=[registry.id,scope?hashSeasonScope(scope):null,itemId,filename,designHash];
   if(image.imageDesignRevision){separated=true;identity.push(image.imageDesignRevision,image.dependencyImages);}
   const taskId=`IMG-${digest(stable(identity)).slice(0,24)}`;
   items.set(asset.id,{itemId,taskId,assetId:asset.id,assetRevision:asset.revision,name:asset.name,key:asset.key,type:asset.type,filename,designHash,design:structuredClone(image.design),origin:structuredClone(asset.origin),dependencies,...(image.imageDesignRevision?{imageDesignRevision:image.imageDesignRevision,dependencyImages:image.dependencyImages}:{}),uses:[],approval:'design-and-uses-approved',imageState:'not-generated'});
  }
  items.get(asset.id).uses.push({episode:use.episode,sceneId:use.sceneId,purpose:use.purpose});
 }
 const queue={version:separated?'series-image-queue-v2':'series-image-queue-v1',sourceHash:registry.sourceHash,assetRegistry:{id:registry.id,revision:registry.revision,snapshot:snapshotName(registry.revision),sha256:loaded.sha256},items:[...items.values()]};
 if(scope){queue.productionScope=structuredClone(scope);queue.productionScopeHash=hashSeasonScope(scope);}
 queue.queueHash=objectHash(queue);return queue;
}

async function main(){
 const [command,...args]=process.argv.slice(2),opts={};
 need(['init','apply','amend','status','sync','prepare-import','record-uses','record-image-design','image-queue'].includes(command)&&args.length%2===0,'USAGE','node scripts/assets.mjs init|apply|amend|status|sync|prepare-import|record-uses|record-image-design|image-queue --registry DIR [--input JSON] [--output RECEIPT] [--source-lock JSON] [--season-scope JSON]');
 for(let i=0;i<args.length;i+=2){need(['--registry','--input','--output','--source-lock','--season-scope'].includes(args[i])&&!Object.hasOwn(opts,args[i])&&text(args[i+1]),'USAGE','未知、重复或缺值参数');opts[args[i]]=args[i+1];}
 need(command==='sync'||(!opts['--source-lock']&&!opts['--season-scope']),'USAGE','源锁与外部季范围参数仅用于sync');
 need(opts['--registry'],'USAGE','缺少 --registry');
 if(command==='status'){need(!opts['--input']&&!opts['--output'],'USAGE','status只读取当前总库');const loaded=await readRegistry(opts['--registry']);console.log(JSON.stringify({ok:true,...loaded}));return;}
 need(opts['--input'],'USAGE','缺少 --input');const bytes=await readFile(opts['--input']),input=JSON.parse(bytes);
 if(command==='image-queue'){
  need(opts['--output'],'USAGE','image-queue须提供独占创建的 --output 清单路径');const queue=await createImageQueue(opts['--registry'],input);
  await mkdir(dirname(resolve(opts['--output'])),{recursive:true});await writeFile(opts['--output'],encode(queue),{flag:'wx'});console.log(JSON.stringify({ok:true,file:resolve(opts['--output']),queueHash:queue.queueHash,items:queue.items.length}));return;
 }
 if(command==='sync'){
  need(opts['--output'],'USAGE','sync须提供不覆盖旧稿的 --output 回执路径');
  const loaded=await readRegistry(opts['--registry']),{checkBundle}=await import('./storyboard.mjs');
  const sourceLock=opts['--source-lock']?JSON.parse(await readFile(opts['--source-lock'],'utf8')):undefined,productionScope=opts['--season-scope']?JSON.parse(await readFile(opts['--season-scope'],'utf8')):loaded.registry.productionScope;
  const report=checkBundle(input,{registry:loaded.registry,sourceLock,seasonScope:productionScope}).assetRegistry;
  const receipt={version:'series-asset-consumption-v1',sourceHash:input.sourceHash,bundleSha256:digest(bytes),registrySnapshot:loaded.file,registrySha256:loaded.sha256,...report};
  if(productionScope)receipt.productionScopeHash=hashSeasonScope(productionScope);
  await mkdir(dirname(resolve(opts['--output'])),{recursive:true});await writeFile(opts['--output'],encode(receipt),{flag:'wx'});console.log(JSON.stringify({ok:true,file:resolve(opts['--output']),closedRequests:report.closedRequests}));return;
 }
 need(!opts['--output'],'USAGE','总库写命令管理版本，不接受 --output');
 console.log(JSON.stringify(await ({init:initializeRegistry,apply:applyChange,amend:amendRequest,'prepare-import':prepareImport,'record-uses':recordAssetUses,'record-image-design':recordImageDesign}[command])(opts['--registry'],input)));
}
if(process.argv[1]&&realpathSync(fileURLToPath(import.meta.url))===realpathSync(process.argv[1]))main().catch(error=>{console.error(`${error.code||'ERROR'}: ${error.message}`);process.exitCode=1;});
