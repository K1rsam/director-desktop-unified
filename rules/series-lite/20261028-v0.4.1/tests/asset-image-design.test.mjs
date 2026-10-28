import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm,readFile,writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';
import {initializeRegistry,readRegistry,recordImageDesign,createImageQueue,checkAssetBindings,applyChange,validateRegistry} from '../scripts/assets.mjs';
import {resolveImageDesign} from '../scripts/asset-image-design.mjs';
const sourceHash='a'.repeat(64);
const design=body=>({state:'TEXT_ADOPTED',text:body,full:{body},detailLevel:'storyboard'});
const asset=(name,dependencies=[])=>({name,key:name+'CH',type:'CH',status:'常态',aliases:[],origin:{kind:'source',sourceRefs:['E1:R1'],basis:'测试源'},design:{...design('同一身份，袖口可抓握，动作条件不变'),dependencies}});
async function setup(t,legacy=false){
 const dir=await mkdtemp(join(tmpdir(),'image-design-'));t.after(()=>rm(dir,{recursive:true,force:true}));
 const assets=[asset('主角'),asset('主角婚礼',[{assetId:'A0001',assetRevision:1,purpose:'同一身份'}]),asset('主角婚礼带Q版',[{assetId:'A0002',assetRevision:1,purpose:'正常婚礼与对应3D Q版同图'}])];
 if(legacy)for(const a of assets)delete a.design.detailLevel;
 await initializeRegistry(dir,{sourceHash,importProfile:'lingmo-v7',assets});return dir;
}
const req=(baseRevision,ids=['A0001'],requestId='image-1')=>({version:'series-image-design-v1',baseRevision,requestId,reason:'补充制图版式，不改分镜必要设定',entries:ids.map(assetId=>({assetId,assetRevision:1,design:{...design('同一身份和动作条件；完整制图外观与展示版式'),detailLevel:'image'},compatibilityBasis:'身份、状态、源定外观、抓握条件和空间均保持，新增制图布局'}))});
const q=(baseRevision,id='A0003')=>({version:'series-image-request-v1',baseRevision,uses:[{assetId:id,assetRevision:1,episode:1,sceneId:'1-1',purpose:'本场所需实际资产'}]});
function bundle(r){return {sourceHash,exportProfile:'lingmo-v7',assetRegistry:{id:r.id,revision:r.revision},assets:r.assets.map(a=>({...a,assetId:a.id,assetRevision:a.revision})),episodes:[{number:1,draft:{units:[{id:'U1',sceneId:'1-1',bindings:r.assets.map(a=>a.key)}]}}]};}

test('必要设计可写分镜；生图独立就绪；反序父子批量不改资产/旧快照/分镜采用',async t=>{
 const dir=await setup(t),old=await readRegistry(dir),bytes=await readFile(old.file),b=bundle(old.registry);
 assert.doesNotThrow(()=>checkAssetBindings(b,old.registry));await assert.rejects(createImageQueue(dir,q(1)),{code:'ASSET_IMAGE_NOT_READY'});
 const input=req(1,['A0003','A0002','A0001']);const result=await recordImageDesign(dir,input),now=await readRegistry(dir);
 assert.equal(result.revision,2);assert.deepEqual(now.registry.assets,old.registry.assets);assert.deepEqual(now.registry.requests,old.registry.requests);assert.deepEqual(now.registry.changes,old.registry.changes);assert.deepEqual(await readFile(old.file),bytes);
 assert.doesNotThrow(()=>checkAssetBindings(b,now.registry));assert.equal((await recordImageDesign(dir,input)).changed,false);
 const queue=await createImageQueue(dir,q(2));assert.equal(queue.version,'series-image-queue-v2');assert.equal(queue.items.length,1);
 const item=queue.items[0];assert.deepEqual(item.dependencyImages.map(p=>p.assetId),['A0002']);assert.deepEqual(item.dependencies.map(p=>p.assetId),['A0002','A0001']);assert.equal(item.imageDesignRevision,1);
});
test('婚礼母制图变更只令Q派生制图待复核；分镜不变，子项重采用后任务身份更新',async t=>{
 const dir=await setup(t);await recordImageDesign(dir,req(1,['A0001','A0002','A0003']));
 const old=await readRegistry(dir),b=bundle(old.registry),before=await createImageQueue(dir,q(2));
 const change=req(2,['A0002'],'wedding-layout');change.entries[0].design.full.body+='；调整同图布局';await recordImageDesign(dir,change);
 const now=await readRegistry(dir);assert.deepEqual(now.registry.assets,old.registry.assets);assert.doesNotThrow(()=>checkAssetBindings(b,now.registry));
 await assert.rejects(createImageQueue(dir,q(3)),{code:'ASSET_IMAGE_DEPENDENCY_STALE'});
 assert.equal((await createImageQueue(dir,q(3,'A0001'))).items.length,1);
 await recordImageDesign(dir,req(3,['A0003'],'q-layout-confirmed'));
 const after=await createImageQueue(dir,q(4));assert.notEqual(after.items[0].taskId,before.items[0].taskId);assert.equal(after.items[0].filename,before.items[0].filename);
});
test('真正分镜约束修订仍拒绝旧分镜与旧制图，不自动续用旧记录',async t=>{
 const dir=await setup(t);await recordImageDesign(dir,req(1,['A0001','A0002','A0003']));const old=(await readRegistry(dir)).registry,b=bundle(old);
 await applyChange(dir,{version:'series-asset-change-v1',baseRevision:2,requestId:'real-change',action:'revise',stage:'asset-design',assetId:'A0002',asset:{type:'CH',design:{...old.assets[1].design,text:'取消可抓握袖口',full:{body:'取消袖口，抓握动作条件发生实质变化'}}},uses:[],reason:'真实动作条件变化',duplicateReview:'同项修订'});
 const now=(await readRegistry(dir)).registry;assert.throws(()=>checkAssetBindings(b,now),{code:'ASSET_REGISTRY_STALE'});assert.throws(()=>resolveImageDesign(now,'A0002'),{code:'ASSET_IMAGE_STALE'});
});
test('旧完整设计保留v1行为；无关项制图不改变旧队列和任务ID',async t=>{
 const dir=await setup(t,true),before=await createImageQueue(dir,q(1,'A0001'));
 assert.equal(before.version,'series-image-queue-v1');assert.equal('imageDesignRevision' in before.items[0],false);
 await recordImageDesign(dir,req(1,['A0003']));const after=await createImageQueue(dir,q(2,'A0001'));
 assert.equal(after.version,'series-image-queue-v1');assert.deepEqual(after.items,before.items);
});
test('旧子项不能静默继承新父制图，需局部重新采用，不能扩大生成项',async t=>{
 const dir=await setup(t,true);await recordImageDesign(dir,req(1));await assert.rejects(createImageQueue(dir,q(2)),{code:'ASSET_IMAGE_DEPENDENCY_STALE'});
 await recordImageDesign(dir,req(2,['A0003','A0002'],'derived'));const queue=await createImageQueue(dir,q(3));assert.equal(queue.items.length,1);
});
test('非法混入、未采用、缺完整体、旧版本/未知ID/重复ID失败均无部分写入',async t=>{
 const dir=await setup(t);
 for(const mutate of [r=>r.assets=[],r=>r.entries[0].status='改身份',r=>r.baseRevision=9,r=>r.entries[0].assetRevision=9,r=>r.entries[0].assetId='A9999',r=>r.entries.push(r.entries[0]),r=>r.entries[0].design.full.body='',r=>r.entries[0].design.state='UNADOPTED',r=>r.entries[0].design.dependencies=[{assetId:'A0003',assetRevision:1,purpose:'偷改父项'}]]){
  const input=req(1);mutate(input);await assert.rejects(recordImageDesign(dir,input));assert.equal((await readRegistry(dir)).registry.revision,1);
 }
 await assert.rejects(recordImageDesign(dir,req(1,['A0003'])),{code:'ASSET_IMAGE_NOT_READY'});
 const reg=(await readRegistry(dir)).registry;reg.assets[0].design.full.body='';assert.throws(()=>validateRegistry(reg),{code:'ASSET_DESIGN_FULL'});
});
test('CLI幂等、同ID不同内容拒绝，当前指针和快照均保持',async t=>{
 const dir=await setup(t),path=join(dir,'input.json'),input=req(1);await writeFile(path,JSON.stringify(input));
 const out=spawnSync(process.execPath,[new URL('../scripts/assets.mjs',import.meta.url).pathname.replace(/^\/([A-Z]:)/i,'$1'),'record-image-design','--registry',dir,'--input',path],{encoding:'utf8'});assert.equal(out.status,0,out.stderr);
 assert.equal((await recordImageDesign(dir,input)).changed,false);input.entries[0].design.text+='不同内容';await assert.rejects(recordImageDesign(dir,input),{code:'ASSET_IMAGE_DESIGN'});assert.equal((await readRegistry(dir)).registry.revision,2);
});
