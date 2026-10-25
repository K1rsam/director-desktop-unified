#!/usr/bin/env node
import {readFile} from 'node:fs/promises';
import {realpathSync} from 'node:fs';
import {resolve,join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {validateSeasonScope,hashSeasonScope,assertSeasonUse} from './season-scope.mjs';
const need=(ok,code,message)=>{if(!ok)throw Object.assign(new Error(message),{code});};
const text=v=>typeof v==='string'&&!!v.trim();
const stable=v=>JSON.stringify(v,(_,x)=>x&&typeof x==='object'&&!Array.isArray(x)?Object.fromEntries(Object.keys(x).sort().map(k=>[k,x[k]])):x);
const digest=bytes=>createHash('sha256').update(bytes).digest('hex');
const hash=v=>digest(stable(v));
const adopted=a=>['TEXT_ADOPTED','REFERENCE_ADOPTED'].includes(a?.design?.state);
const sceneKey=u=>JSON.stringify([u?.episode,u?.sceneId]);
const prefix={CH:'角色',SC:'场景',PR:'道具'};

// Read-only: this validator never generates images, adopts candidates or edits a queue.
export function validateImageBatch(queue,batch,{registry,sourceHash,snapshotSha256}={}){
 need(queue?.version==='series-image-queue-v1'&&Array.isArray(queue.items)&&queue.items.length>0,'IMAGE_QUEUE','缺少显式图片清单');
 const {queueHash,...payload}=queue;need(/^[a-f0-9]{64}$/.test(queueHash)&&hash(payload)===queueHash,'IMAGE_QUEUE_HASH','图片清单hash已变化');
 need(registry?.version==='series-assets-v1'&&registry.id===queue.assetRegistry?.id&&registry.sourceHash===queue.sourceHash&&Number.isSafeInteger(queue.assetRegistry.revision)&&queue.assetRegistry.revision<=registry.revision,'IMAGE_REGISTRY','库身份、源或版本与图片清单不符');
 need(snapshotSha256===queue.assetRegistry.sha256,'IMAGE_REGISTRY_HASH','清单采用的不可变总库快照hash不匹配');
 const scoped=registry.productionScope!==undefined||queue.productionScope!==undefined||registry.productionScopeHash!==undefined||queue.productionScopeHash!==undefined;
 let scope;
 if(scoped){
  need(registry.productionScope&&queue.productionScope,'IMAGE_SCOPE_BINDING','季模式不能省略总库或清单productionScope后降级');
  scope=validateSeasonScope(queue.productionScope,{sourceHash:queue.sourceHash});
  validateSeasonScope(registry.productionScope,{sourceHash:registry.sourceHash});
  need(hashSeasonScope(scope)===queue.productionScopeHash&&hashSeasonScope(registry.productionScope)===registry.productionScopeHash&&queue.productionScopeHash===registry.productionScopeHash,'IMAGE_SCOPE_HASH','季范围hash或当前总库范围不一致');
  need(sourceHash===queue.sourceHash,'IMAGE_SOURCE_HASH','源文件字节hash与本季图片清单不同');
 }
 need(batch?.version==='series-image-batch-v1'&&batch.queueHash===queueHash&&batch.sourceHash===queue.sourceHash&&Array.isArray(batch.tasks)&&batch.tasks.length>0,'IMAGE_BATCH','本批须绑定同一清单、源文件与显式tasks');
 need(scoped?batch.productionScopeHash===queue.productionScopeHash:batch.productionScopeHash===undefined,'IMAGE_SCOPE_HASH','本批缺少/错用季范围hash，不能退回旧模式');
 const ids=new Map(registry.assets.map(a=>[a.id,a])),items=new Map(),taskIds=new Set();
 for(const item of queue.items){
  need(item&&text(item.itemId)&&!items.has(item.itemId)&&/^A\d{4,}$/.test(item.assetId)&&item.itemId===`${item.assetId}@${item.assetRevision}`&&Number.isSafeInteger(item.assetRevision)&&item.assetRevision>0,'IMAGE_ITEM','图片项的稳定ID或设计版本无效');
  need(item.approval==='design-and-uses-approved'&&item.imageState==='not-generated'&&adopted(item)&&item.designHash===hash(item.design),'IMAGE_DESIGN_HASH','清单必须保留已采用实际设计，不能仅改设计版本或将候选当正式图');
  need(Array.isArray(item.uses)&&item.uses.length>0&&item.uses.every(u=>u&&Number.isInteger(u.episode)&&text(u.sceneId)&&text(u.purpose)),'IMAGE_USES','图片项缺实际源用途');
  if(scope)for(const use of item.uses)assertSeasonUse(scope,use,`${item.itemId} 图片清单用途`);
  const expectedTask=`IMG-${hash([registry.id,queue.productionScopeHash??null,item.itemId,item.filename,item.designHash]).slice(0,24)}`;
  need(item.taskId===expectedTask&&!taskIds.has(item.taskId),'IMAGE_TASK_ID','图片任务ID与身份、设计版本、范围及文件名不符');
  taskIds.add(item.taskId);items.set(item.itemId,item);
 }
 const seen=new Set();
 for(const task of batch.tasks){
  const item=items.get(task?.itemId);need(item&&!seen.has(item.itemId),'IMAGE_TASK_ID','本批含未知或重复图片任务');seen.add(item.itemId);
  for(const field of ['taskId','assetId','assetRevision','designHash','filename'])need(task[field]===item[field],'IMAGE_TASK_BINDING',`${item.itemId} 的${field}不对应清单任务`);
  need(Array.isArray(task.uses)&&task.uses.length>0&&task.uses.every(u=>item.uses.some(v=>stable(v)===stable(u))),'IMAGE_TASK_USES',`${item.itemId} 的本批用途不在显式图片清单`);
  if(scope)for(const use of task.uses)assertSeasonUse(scope,use,`${item.itemId} 本批用途`);
  const asset=ids.get(item.assetId);
  need(asset&&asset.revision===item.assetRevision&&adopted(asset)&&hash(asset.design)===item.designHash,'IMAGE_DESIGN_STALE',`${item.itemId} 的当前实际设计或采用版本已变化`);
  need(asset.imageRequired!==false,'IMAGE_NOT_REQUIRED',`${item.itemId} 已登记不生图`);
  const filename=`${prefix[asset.type]}-${registry.importProfile==='lingmo-v7'?asset.importName:asset.name}.png`;
  need(prefix[asset.type]&&item.filename===filename&&!/[\\/\x00-\x1f:*?"<>|]/.test(filename)&&Buffer.byteLength(filename,'utf8')<=255,'IMAGE_FILENAME',`${item.itemId} 的精确文件名与当前库不同或不安全`);
  need(item.key===asset.key&&item.name===asset.name&&item.type===asset.type&&stable(item.origin)===stable(asset.origin),'IMAGE_IDENTITY',`${item.itemId} 的身份或源材料登记已变化`);
  if(scope)need(Array.isArray(asset.scopeUses)&&item.uses.every(u=>asset.scopeUses.some(v=>sceneKey(v)===sceneKey(u))),'IMAGE_SOURCE_USES',`${item.itemId} 有未在资产登记的源用途`);
  const dependencies=[],visited=new Set(),active=new Set();
  const collect=current=>{
   need(!active.has(current.id),'IMAGE_DESIGN_DEPENDENCY','设计依赖存在循环');active.add(current.id);
   for(const d of current.design.dependencies??[]){
    const parent=ids.get(d.assetId);need(parent&&parent.revision===d.assetRevision&&adopted(parent),'IMAGE_DESIGN_DEPENDENCY',`${item.itemId} 的祖先设计缺失、未采用或版本已过期`);
    need(!active.has(parent.id),'IMAGE_DESIGN_DEPENDENCY','设计依赖存在循环');
    if(visited.has(parent.id))continue;visited.add(parent.id);dependencies.push({...d,designHash:hash(parent.design),designState:parent.design.state});collect(parent);
   }
   active.delete(current.id);
  };collect(asset);
  need(stable(dependencies)===stable(item.dependencies),'IMAGE_DESIGN_DEPENDENCY',`${item.itemId} 的继承依赖与当前库不同`);
 }
 return {ok:true,legacy:!scoped,queueHash,productionScopeHash:queue.productionScopeHash,tasks:batch.tasks.length,imageApproval:false,proof:'仅核显式批次与本季范围、源hash、设计版本及文件名；不代表图像质量、用户采用或生图授权'};
}

async function main(){
 const args=process.argv.slice(2),opts={};need(args.length%2===0,'IMAGE_USAGE','--queue JSON --batch JSON --registry DIR --source 原件副本');
 for(let i=0;i<args.length;i+=2){need(['--queue','--batch','--registry','--source'].includes(args[i])&&!Object.hasOwn(opts,args[i])&&text(args[i+1]),'IMAGE_USAGE','未知、重复或缺值参数');opts[args[i]]=args[i+1];}
 need(opts['--queue']&&opts['--batch']&&opts['--registry'],'IMAGE_USAGE','缺少清单、本批任务或当前总库');
 const queue=JSON.parse(await readFile(opts['--queue'],'utf8')),batch=JSON.parse(await readFile(opts['--batch'],'utf8')),root=resolve(opts['--registry']);
 const head=JSON.parse(await readFile(join(root,'current.json'),'utf8'));
 need(head.version==='series-assets-head-v1'&&Number.isSafeInteger(head.revision)&&head.revision>0,'IMAGE_REGISTRY','当前总库指针无效');
 const bytes=await readFile(join(root,`v${String(head.revision).padStart(6,'0')}.json`));need(digest(bytes)===head.sha256,'IMAGE_REGISTRY_HASH','当前总库快照被原地改写');
 const registry=JSON.parse(bytes);need(registry.id===head.id&&registry.revision===head.revision,'IMAGE_REGISTRY','当前指针与总库身份/版本不符');
 need(queue.assetRegistry?.snapshot===`v${String(queue.assetRegistry?.revision).padStart(6,'0')}.json`,'IMAGE_REGISTRY','清单快照路径须为对应不可变版本名');
 const snapshotBytes=await readFile(join(root,queue.assetRegistry.snapshot)),snapshotSha256=digest(snapshotBytes);
 const snapshot=JSON.parse(snapshotBytes);need(snapshot.id===registry.id&&snapshot.revision===queue.assetRegistry.revision&&snapshot.sourceHash===queue.sourceHash,'IMAGE_REGISTRY','原采用快照与本清单不符');
 need(!registry.productionScope||opts['--source'],'IMAGE_SOURCE_REQUIRED','季模式每批须提供sourceHash对应的原始源文件副本');
 const sourceHash=opts['--source']?digest(await readFile(opts['--source'])):undefined;
 console.log(JSON.stringify(validateImageBatch(queue,batch,{registry,sourceHash,snapshotSha256})));
}
if(process.argv[1]&&realpathSync(fileURLToPath(import.meta.url))===realpathSync(process.argv[1]))main().catch(error=>{console.error(`${error.code||'ERROR'}: ${error.message}`);process.exitCode=1;});
