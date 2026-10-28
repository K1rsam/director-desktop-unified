// Image-only design records never mutate narrative asset rows or their revisions.
import {createHash} from 'node:crypto';
const need=(ok,code,message)=>{if(!ok)throw Object.assign(new Error(message),{code});};
const text=v=>typeof v==='string'&&!!v.trim();
const positive=v=>Number.isSafeInteger(v)&&v>0;
const fields=(v,allowed)=>v&&typeof v==='object'&&!Array.isArray(v)&&Object.keys(v).every(k=>allowed.includes(k));
const adopted=d=>['TEXT_ADOPTED','REFERENCE_ADOPTED'].includes(d?.state);
export const stableImageValue=v=>JSON.stringify(v,(_,x)=>x&&typeof x==='object'&&!Array.isArray(x)?Object.fromEntries(Object.keys(x).sort().map(k=>[k,x[k]])):x);
export const imageHash=v=>createHash('sha256').update(stableImageValue(v)).digest('hex');
const entries=r=>(r.imageDesigns??[]).flatMap(b=>b.entries);
const same=(a,b)=>stableImageValue(a)===stableImageValue(b);

export function validateDesignLevel(asset){
 const d=asset.design;
 need(d.detailLevel===undefined||['storyboard','image'].includes(d.detailLevel),'ASSET_DESIGN_LEVEL',`${asset.id} 的detailLevel须为storyboard或image`);
 if(d.detailLevel!==undefined&&adopted(d))need(text(d.full?.body),'ASSET_DESIGN_FULL',`${asset.id} 须有必要完整设计full.body，不能只给名称或短卡`);
}

export function validateImageDesigns(registry){
 need(registry.imageDesigns===undefined||Array.isArray(registry.imageDesigns),'ASSET_IMAGE_DESIGN','imageDesigns须为批次数组');
 const ids=new Map(registry.assets.map(a=>[a.id,a])),seen=new Set(),versions=new Map(),history=new Map();
 for(const batch of registry.imageDesigns??[]){
  need(fields(batch,['id','reason','recordedAtRevision','inputHash','entries'])&&text(batch.id)&&!seen.has(batch.id)&&text(batch.reason)&&positive(batch.recordedAtRevision)&&batch.recordedAtRevision<=registry.revision&&/^[a-f0-9]{64}$/.test(batch.inputHash)&&Array.isArray(batch.entries)&&batch.entries.length>0,'ASSET_IMAGE_DESIGN','制图批次缺少唯一标识、版本或实际条目');
  need(![...(registry.requests??[]),...(registry.changes??[]),...(registry.usageAnnotations??[])].some(r=>r.id===batch.id),'ASSET_IMAGE_DESIGN','制图requestId与其它修改冲突');
  seen.add(batch.id);const inBatch=new Set();
  for(const e of batch.entries){
   const asset=ids.get(e?.assetId);
   need(fields(e,['assetId','assetRevision','imageDesignRevision','design','dependencyImages','compatibilityBasis'])&&asset&&!inBatch.has(e.assetId)&&positive(e.assetRevision)&&e.assetRevision<=asset.revision&&e.imageDesignRevision===(versions.get(e.assetId)??0)+1&&text(e.compatibilityBasis),'ASSET_IMAGE_DESIGN','制图条目未知、重复、版本断裂或缺少保留分镜约束的依据');
   need(adopted(e.design)&&e.design.detailLevel==='image'&&text(e.design.text)&&text(e.design.full?.body),'ASSET_IMAGE_DESIGN','制图记录须为已采用的完整制图设计');
   need(Array.isArray(e.dependencyImages),'ASSET_IMAGE_DESIGN','缺少直接父制图版本');
   const parents=new Set();
   for(const p of e.dependencyImages){
    need(fields(p,['assetId','assetRevision','imageDesignRevision','designHash'])&&ids.has(p.assetId)&&p.assetId!==e.assetId&&!parents.has(p.assetId)&&positive(p.assetRevision)&&p.assetRevision<=ids.get(p.assetId).revision&&Number.isSafeInteger(p.imageDesignRevision)&&p.imageDesignRevision>=0&&/^[a-f0-9]{64}$/.test(p.designHash),'ASSET_IMAGE_DESIGN','父制图绑定无效');parents.add(p.assetId);
   }
   inBatch.add(e.assetId);versions.set(e.assetId,e.imageDesignRevision);history.set(`${e.assetId}@${e.imageDesignRevision}`,e);
  }
 }
 for(const e of entries(registry))for(const p of e.dependencyImages)if(p.imageDesignRevision>0){
  const parent=history.get(`${p.assetId}@${p.imageDesignRevision}`);
  need(parent&&parent.assetRevision===p.assetRevision&&imageHash(parent.design)===p.designHash,'ASSET_IMAGE_DESIGN','父制图记录不存在或hash不匹配');
 }
}

// Legacy unmarked designs retain their old contract. Marked storyboard designs
// cannot masquerade as production-ready art. Parent image changes stay local.
export function resolveImageDesign(registry,assetId,cache=new Map(),active=new Set()){
 if(cache.has(assetId))return cache.get(assetId);
 need(!active.has(assetId),'ASSET_IMAGE_CYCLE','制图依赖成环');active.add(assetId);
 const asset=registry.assets.find(a=>a.id===assetId);
 need(asset&&adopted(asset.design),'ASSET_DESIGN_UNADOPTED',`${assetId} 分镜必要设计未采用`);
 const all=entries(registry).filter(e=>e.assetId===assetId),record=all.at(-1);
 if(record)need(record.assetRevision===asset.revision,'ASSET_IMAGE_STALE',`${assetId} 的分镜设计已变；只复核该项制图资料`);
 const design=record?.design??asset.design;
 need(record||design.detailLevel!=='storyboard','ASSET_IMAGE_NOT_READY',`${assetId} 仅有分镜必要设计，尚无可生图完整设计；分镜仍可推进`);
 const declared=asset.design.dependencies??[],parents=[];
 if(record)need(same(design.dependencies??[],declared)&&same(record.dependencyImages.map(p=>p.assetId).sort(),declared.map(p=>p.assetId).sort()),'ASSET_IMAGE_DEPENDENCY','制图记录须绑定实际直接父设计');
 for(const d of declared){
  const parent=resolveImageDesign(registry,d.assetId,cache,active);
  need(parent.assetRevision===d.assetRevision,'ASSET_DESIGN_DEPENDENCY_STALE',`${assetId} 的父分镜设计已变`);
  const expected=record?.dependencyImages.find(p=>p.assetId===d.assetId)??{assetRevision:d.assetRevision,imageDesignRevision:0,designHash:imageHash(registry.assets.find(a=>a.id===d.assetId).design)};
  need(expected.assetRevision===parent.assetRevision&&expected.imageDesignRevision===parent.imageDesignRevision&&expected.designHash===parent.designHash,'ASSET_IMAGE_DEPENDENCY_STALE',`${assetId} 的父制图 ${d.assetId} 已变；仅复核真实派生制图，不回退分镜`);
  parents.push({assetId:d.assetId,assetRevision:parent.assetRevision,imageDesignRevision:parent.imageDesignRevision,designHash:parent.designHash});
 }
 active.delete(assetId);
 const result={assetId,assetRevision:asset.revision,imageDesignRevision:record?.imageDesignRevision??0,design,designHash:imageHash(design),dependencyImages:parents};cache.set(assetId,result);return result;
}

// One batch; sort by the existing narrative dependency graph, not caller order.
export function prepareImageDesignBatch(registry,input){
 need(fields(input,['version','baseRevision','requestId','reason','entries'])&&input.version==='series-image-design-v1'&&positive(input.baseRevision)&&text(input.requestId)&&text(input.reason)&&Array.isArray(input.entries)&&input.entries.length>0,'ASSET_IMAGE_DESIGN','制图补充须有批次版本、原因与条目');
 const pending=new Map(),existing=entries(registry),ids=new Map(registry.assets.map(a=>[a.id,a]));
 for(const e of input.entries){
  const a=ids.get(e?.assetId);
  need(fields(e,['assetId','assetRevision','design','compatibilityBasis'])&&a&&!pending.has(e.assetId)&&e.assetRevision===a.revision&&adopted(a.design)&&text(e.compatibilityBasis),'ASSET_IMAGE_DESIGN','制图补充须绑定当前采用资产，不接受资产身份、版本或范围修改');
  need(adopted(e.design)&&text(e.design.text)&&text(e.design.full?.body)&&(e.design.detailLevel===undefined||e.design.detailLevel==='image'),'ASSET_IMAGE_DESIGN','须提供完整已采用制图文字与full.body');
  need(e.design.dependencies===undefined||same(e.design.dependencies,a.design.dependencies??[]),'ASSET_IMAGE_DEPENDENCY','制图不能另改分镜父设计链');
  pending.set(e.assetId,e);
 }
 const batch={id:input.requestId,reason:input.reason,recordedAtRevision:registry.revision+1,inputHash:imageHash({reason:input.reason,entries:input.entries}),entries:[]};
 const trial={...registry,imageDesigns:[...(registry.imageDesigns??[]),batch]},done=new Set(),active=new Set();
 const add=id=>{
  if(done.has(id))return;need(!active.has(id),'ASSET_IMAGE_CYCLE','制图依赖成环');active.add(id);
  const e=pending.get(id),asset=ids.get(id),parents=[];
  for(const d of asset.design.dependencies??[]){
   if(pending.has(d.assetId))add(d.assetId);
   const parent=resolveImageDesign(trial,d.assetId);
   need(parent.assetRevision===d.assetRevision,'ASSET_DESIGN_DEPENDENCY_STALE',`${id} 的父分镜设计已变，先修真实约束`);
   parents.push({assetId:d.assetId,assetRevision:parent.assetRevision,imageDesignRevision:parent.imageDesignRevision,designHash:parent.designHash});
  }
  const design={...structuredClone(e.design),detailLevel:'image',dependencies:structuredClone(asset.design.dependencies??[])};
  batch.entries.push({assetId:id,assetRevision:asset.revision,imageDesignRevision:(existing.filter(x=>x.assetId===id).at(-1)?.imageDesignRevision??0)+1,design,dependencyImages:parents,compatibilityBasis:e.compatibilityBasis});
  active.delete(id);done.add(id);
 };
 for(const id of pending.keys())add(id);
 return batch;
}
