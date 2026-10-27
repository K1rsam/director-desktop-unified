import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm,readFile,writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {initializeRegistry,readRegistry,recordAssetUses,createImageQueue,applyChange} from '../scripts/assets.mjs';
import {assetScopeUses,validateUsageAnnotations} from '../scripts/asset-uses.mjs';
const base=fileURLToPath(new URL('..',import.meta.url)),sourceHash='a'.repeat(64);
const use=(episode=1)=>({episode,sceneId:`${episode}-1`});
const asset=(name)=>({name,key:name+'CH',type:'CH',status:'常态',aliases:[],scopeUses:[use()],origin:{kind:'source',sourceRefs:['E01:R1'],basis:'合成源身份'},design:{state:'TEXT_ADOPTED',text:'灰衣、短发、同一身份，完整采用测试设计',full:{body:'同一灰衣结构和常态身份'},dependencies:[]}});
const request=(revision=1)=>({version:'series-asset-uses-v1',baseRevision:revision,requestId:'later-uses',reason:'后续源仍为同一常态，不改外形结构或状态限制',entries:[{assetId:'A0001',assetRevision:1,uses:[{...use(2),purpose:'后集身份复用',sourceRefs:['E02:R1']}]}]});
const queueRequest=(revision=1,episode=1)=>({version:'series-image-request-v1',baseRevision:revision,uses:[{assetId:'A0001',assetRevision:1,...use(episode),purpose:'合成身份板'}]});
async function setup(t,scoped=false){
 const root=await mkdtemp(join(tmpdir(),'asset-uses-'));t.after(()=>rm(root,{recursive:true,force:true}));
 const input={sourceHash,importProfile:'lingmo-v7',assets:[asset('角色'),{...asset('衍生'),design:{...asset('衍生').design,dependencies:[{assetId:'A0001',assetRevision:1,purpose:'继承脸型'}]}}]};
 if(scoped)input.productionScope={version:'series-season-scope-v1',id:'s1',revision:1,seasonNumber:1,status:'locked',sourceHash,readingScope:[{number:1,sceneIds:['1-1']},{number:2,sceneIds:['2-1']},{number:3,sceneIds:['3-1']}],episodes:[{number:1,sceneIds:['1-1']},{number:2,sceneIds:['2-1']}],target:{minSeconds:7200,maxSeconds:9000,planningSeconds:8100},ending:{episode:2,sceneId:'2-1',sourceRef:'E02:R1',closure:'合成季终',hook:'保留后续',estimate:{minSeconds:7200,maxSeconds:9000}},confirmation:{kind:'delegated',basis:'合成测试'}};
 await initializeRegistry(root,input);return root;
}
test('一次批量用途补登不改任何资产、父依赖、请求和既有文件名/设计hash/任务ID',async t=>{
 const root=await setup(t),old=await readRegistry(root),oldBytes=await readFile(old.file),queue=await createImageQueue(root,queueRequest());
 const input=request();input.entries.push({assetId:'A0002',assetRevision:1,uses:[{...use(2),purpose:'同版衍生复用',sourceRefs:['E02:R2']}]});
 const result=await recordAssetUses(root,input),now=await readRegistry(root);assert.equal(result.revision,2);assert.equal(result.entries,2);
 assert.deepEqual(now.registry.assets,old.registry.assets);assert.deepEqual(now.registry.requests,old.registry.requests);assert.deepEqual(now.registry.changes,old.registry.changes);assert.deepEqual(await readFile(old.file),oldBytes);
 const later=await createImageQueue(root,queueRequest(2,2));for(const k of ['taskId','filename','designHash','assetRevision'])assert.equal(queue.items[0][k],later.items[0][k]);
 assert.deepEqual(assetScopeUses(now.registry,now.registry.assets[0]).map(u=>u.episode),[1,2]);
 const repeat=await recordAssetUses(root,input);assert.equal(repeat.changed,false);assert.equal(repeat.revision,2);
});
test('真实CLI写一次新快照；同内容新requestId零写，混合重复后可原请求重试',async t=>{
 const root=await setup(t),file=join(root,'uses.json');await writeFile(file,JSON.stringify(request()));
 const cli=spawnSync(process.execPath,[join(base,'scripts/assets.mjs'),'record-uses','--registry',root,'--input',file],{encoding:'utf8'});assert.equal(cli.status,0,cli.stderr);
 const repeated={...request(2),requestId:'repeat'};assert.equal((await recordAssetUses(root,repeated)).changed,false);
 repeated.requestId='mixed';repeated.entries[0].uses.push({...use(3),purpose:'第三集复用',sourceRefs:['E03:R1']});
 assert.equal((await recordAssetUses(root,repeated)).revision,3);assert.equal((await recordAssetUses(root,repeated)).changed,false);
});
test('季内新增用途可生成队列；不借登记扩大到下一季',async t=>{
 const root=await setup(t,true);await assert.rejects(createImageQueue(root,queueRequest(1,2)),{code:'ASSET_SCOPE_USES'});
 await recordAssetUses(root,request());assert.equal((await createImageQueue(root,queueRequest(2,2))).items.length,1);
 const future=request(2);future.requestId='future';future.entries[0].uses[0]={...future.entries[0].uses[0],...use(3)};
 await assert.rejects(recordAssetUses(root,future),{code:'SEASON_OUT_OF_SCOPE'});assert.equal((await readRegistry(root)).registry.revision,2);
});
test('拒绝设计偷改、未采用、旧版、缺源、未知ID、批内冲突，失败不部分写入',async t=>{
 for(const mode of ['design','missing-ref','old-base','bad-id','old-design','duplicate','unitId']){
  const root=await setup(t),input=request();
  if(mode==='design')input.entries[0].design={text:'偷换服装'};
  if(mode==='missing-ref')input.entries[0].uses[0].sourceRefs=[];
  if(mode==='old-base')input.baseRevision=7;
  if(mode==='bad-id')input.entries.push({...input.entries[0],assetId:'A9999'});
  if(mode==='old-design')input.entries[0].assetRevision=2;
  if(mode==='duplicate')input.entries.push(input.entries[0]);
  if(mode==='unitId')input.entries[0].uses[0].unitId='fake';
  await assert.rejects(recordAssetUses(root,input));assert.equal((await readRegistry(root)).registry.revision,1);
 }
 const root=await setup(t),{registry}=await readRegistry(root);await applyChange(root,{version:'series-asset-change-v1',baseRevision:1,requestId:'unadopt',action:'revise',stage:'asset-design',assetId:'A0001',asset:{...registry.assets[0],design:{state:'UNADOPTED',text:''}},reason:'候选不能复用',duplicateReview:'同一身份',uses:[]});
 const input=request(2);input.entries[0].assetRevision=2;await assert.rejects(recordAssetUses(root,input),{code:'ASSET_USAGE'});
});
test('新服装独立ID不废旧设计；真正改母设计仍拦截旧设计/衍生，旧用途不自动继承到新版',async t=>{
 const root=await setup(t,true);await recordAssetUses(root,request());const old=(await readRegistry(root)).registry;
 await applyChange(root,{version:'series-asset-change-v1',baseRevision:2,requestId:'new-costume',action:'add',stage:'asset-design',asset:asset('有袖外套'),scopeUses:[use(2)],reason:'新的正式服装状态',duplicateReview:'独立状态',uses:[]});
 assert.equal((await createImageQueue(root,queueRequest(3,2))).items[0].assetRevision,1);
 const parent=structuredClone(old.assets[0]);parent.design.text+=' 红色铠甲';
 await applyChange(root,{version:'series-asset-change-v1',baseRevision:3,requestId:'body-change',action:'revise',stage:'asset-design',assetId:parent.id,asset:parent,scopeUses:[use()],reason:'真实造型修改',duplicateReview:'同项修订',uses:[]});
 await assert.rejects(createImageQueue(root,queueRequest(4,1)),{code:'ASSET_REGISTRY_STALE'});
 const q=queueRequest(4,1);q.uses[0].assetId='A0002';await assert.rejects(createImageQueue(root,q),{code:'ASSET_DESIGN_DEPENDENCY_STALE'});
 const now=(await readRegistry(root)).registry;assert.deepEqual(assetScopeUses(now,now.assets[0]).map(u=>u.episode),[1]);
 assert.doesNotThrow(()=>validateUsageAnnotations(now));
});
