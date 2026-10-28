// Additive source-use evidence is separate from adopted design and its revision.
import {assertSeasonUse} from './season-scope.mjs';
const need=(ok,message)=>{if(!ok)throw Object.assign(new Error(message),{code:'ASSET_USAGE'});};
const text=v=>typeof v==='string'&&!!v.trim()&&!/[\0\r]/.test(v);
const positive=v=>Number.isSafeInteger(v)&&v>0;
const fields=(v,allowed)=>v&&typeof v==='object'&&!Array.isArray(v)&&Object.keys(v).every(k=>allowed.includes(k));
const sceneKey=u=>JSON.stringify([u.episode,u.sceneId]);

export function validateUsageEntries(entries,registry,{current=false}={}){
 const ids=new Map(registry.assets.map(a=>[a.id,a])),seen=new Set();
 need(Array.isArray(entries)&&entries.length>0,'用途登记须有非空entries');
 for(const entry of entries){
  need(fields(entry,['assetId','assetRevision','uses']),'用途项仅接受assetId/assetRevision/uses，不能混入设计修订');
  const asset=ids.get(entry.assetId);
  need(asset&&!seen.has(entry.assetId)&&positive(entry.assetRevision)&&entry.assetRevision<=asset.revision,'用途项ID重复、未知或版本无效');seen.add(entry.assetId);
  if(current)need(entry.assetRevision===asset.revision&&['TEXT_ADOPTED','REFERENCE_ADOPTED'].includes(asset.design.state),'新增用途须引用当前已采用设计，不能借登记采用候选或升级旧设计');
  need(Array.isArray(entry.uses)&&entry.uses.length>0,'用途项须有uses');
  const scenes=new Set();
  for(const use of entry.uses){
   need(fields(use,['episode','sceneId','purpose','sourceRefs'])&&Number.isSafeInteger(use.episode)&&text(use.sceneId)&&text(use.purpose),'用途须有真实集场和purpose，不接受伪造unitId或其它字段');
   need(Array.isArray(use.sourceRefs)&&use.sourceRefs.length>0&&use.sourceRefs.every(text)&&new Set(use.sourceRefs).size===use.sourceRefs.length,'用途须有不重复的源引用');
   need(!scenes.has(sceneKey(use)),'同资产同集场的用途请合成一项');scenes.add(sceneKey(use));
   if(registry.productionScope)assertSeasonUse(registry.productionScope,use,'新增资产用途');
  }
 }
}

export function validateUsageAnnotations(registry){
 if(registry.usageAnnotations===undefined)return;
 need(Array.isArray(registry.usageAnnotations),'usageAnnotations须为数组');
 const seen=new Set();
 for(const row of registry.usageAnnotations){
  need(fields(row,['id','reason','recordedAtRevision','entries'])&&text(row.id)&&text(row.reason)&&positive(row.recordedAtRevision)&&row.recordedAtRevision<=registry.revision&&!seen.has(row.id),'用途记录缺少唯一ID、原因或有效库版本');seen.add(row.id);
  validateUsageEntries(row.entries,registry);
 }
}

// Callers read a validated registry. An annotation never migrates to a new design revision.
export function assetScopeUses(registry,asset){
 const uses=[...(asset.scopeUses??[])];
 for(const row of registry.usageAnnotations??[])for(const entry of row.entries){
  if(entry.assetId===asset.id&&entry.assetRevision===asset.revision)uses.push(...entry.uses);
 }
 const seen=new Set();
 return uses.filter(u=>{const key=sceneKey(u);if(seen.has(key))return false;seen.add(key);return true;});
}
