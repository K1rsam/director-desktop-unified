import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm,readFile,writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import * as assets from '../scripts/assets.mjs';
const base=fileURLToPath(new URL('..',import.meta.url)),sourceHash='a'.repeat(64);
const scope=()=>({version:'series-season-scope-v1',id:'season-1',revision:1,seasonNumber:1,status:'locked',sourceHash,readingScope:[{number:1,sceneIds:['1-1']},{number:2,sceneIds:['2-1']}],episodes:[{number:1,sceneIds:['1-1']}],target:{minSeconds:7200,maxSeconds:9000,planningSeconds:8100},ending:{episode:1,sceneId:'1-1',sourceRef:'source-body.txt:L1-L6',closure:'本季问题闭合',hook:'后续悬念保留',estimate:{minSeconds:7200,maxSeconds:9000}},confirmation:{kind:'delegated',basis:'合成测试已明确委托'}});
const use=()=>({episode:1,sceneId:'1-1'});
const asset=(name='普通形态')=>({name,key:name+'CH',type:'CH',status:'常态',aliases:[],scopeUses:[use()],origin:{kind:'source',sourceRefs:['E01:R1'],basis:'合成原文明确出场'},design:{state:'TEXT_ADOPTED',text:'源内常态的完整设计。',full:{appearance:'同一合成身份'}}});
const seed=()=>({sourceHash,productionScope:scope(),assets:[asset()]});
async function sandbox(t){const root=await mkdtemp(join(tmpdir(),'season-assets-'));t.after(()=>rm(root,{recursive:true,force:true}));return root;}
const change=()=>({version:'series-asset-change-v1',baseRevision:1,requestId:'R1',stage:'asset-design',action:'add',reason:'当前季实际需要',duplicateReview:'与现有资产核重后为独立状态',uses:[],scopeUses:[use()],asset:asset('新服装')});
const cli=args=>spawnSync(process.execPath,[join(base,'scripts/assets.mjs'),...args],{encoding:'utf8'});
const save=async(root,name,input)=>{const file=join(root,name);await writeFile(file,JSON.stringify(input));return file;};

test('季库init锁定productionScope与原sourceHash，不静默丢弃范围',async t=>{const root=await sandbox(t),input=seed();await assets.initializeRegistry(root,input);const {registry}=await assets.readRegistry(root);assert.deepEqual(registry.productionScope,input.productionScope);assert.match(registry.productionScopeHash,/^[a-f0-9]{64}$/);});
test('季库初始未来形态和无法归属的资产在写入前被拒绝',async t=>{for(const mode of ['future','missing','unlocked']){const root=join(await sandbox(t),mode),input=seed();if(mode==='future')input.assets[0].scopeUses=[{episode:2,sceneId:'2-1'}];if(mode==='missing')delete input.assets[0].scopeUses;if(mode==='unlocked')input.productionScope.status='proposed';await assert.rejects(assets.initializeRegistry(root,input));await assert.rejects(readFile(join(root,'current.json')),{code:'ENOENT'});}});
test('asset-design实际写入拒绝未来服装的scopeUses，拒绝省略范围用途',async t=>{const root=await sandbox(t);await assets.initializeRegistry(root,seed());for(const mode of ['future','missing']){const input=change();if(mode==='future')input.scopeUses=[{episode:2,sceneId:'2-1'}];else delete input.scopeUses;await assert.rejects(assets.applyChange(root,input));assert.equal((await assets.readRegistry(root)).registry.revision,1);}});
test('production和amend不能把已登记资产调到下一季',async t=>{const root=await sandbox(t);await assets.initializeRegistry(root,seed());const input={...change(),stage:'production',action:'reuse',assetId:'A0001',uses:[{episode:2,sceneId:'2-1',unitId:'U2'}]};delete input.asset;await assert.rejects(assets.applyChange(root,input),{code:'SEASON_OUT_OF_SCOPE'});input.uses=[{episode:1,sceneId:'1-1',unitId:'U1'}];await assets.applyChange(root,input);await assert.rejects(assets.amendRequest(root,{version:'series-asset-amend-v1',baseRevision:2,requestId:'R1',action:'retarget',uses:[{episode:2,sceneId:'2-1',unitId:'U2'}],reason:'企图提前到下一季',sourceReview:'合成源坐标'}),{code:'SEASON_OUT_OF_SCOPE'});assert.equal((await assets.readRegistry(root)).registry.revision,2);});

const imageRequest=()=>({version:'series-image-request-v1',baseRevision:1,uses:[{assetId:'A0001',assetRevision:1,...use(),purpose:'本季画面实际出场，需要身份板'}]});
test('图片清单只由实际uses产生，带源用途、设计版本和精确文件名',async t=>{
 const root=await sandbox(t),input=seed();input.importProfile='lingmo-v7';input.assets.push({...asset('仅声源'),imageRequired:false});
 await assets.initializeRegistry(root,input);assert.equal(typeof assets.createImageQueue,'function');
 const queue=await assets.createImageQueue(root,imageRequest());
 assert.equal(queue.items.length,1);assert.equal(queue.items[0].assetId,'A0001');assert.equal(queue.items[0].filename,'角色-普通形态.png');assert.equal(queue.items[0].assetRevision,1);assert.deepEqual(queue.items[0].uses,imageRequest().uses.map(({assetId,assetRevision,...u})=>u));
 assert.deepEqual(queue.productionScope,input.productionScope);assert.match(queue.queueHash,/^[a-f0-9]{64}$/);assert.match(queue.items[0].designHash,/^[a-f0-9]{64}$/);assert.equal(queue.items[0].imageState,'not-generated');
});
test('图片清单拒绝下一季用途、未采用设计、过期依赖和过期总库版本',async t=>{
 for(const mode of ['future','unadopted','dependency','registry','missing-source-use']){
  const root=await sandbox(t),input=seed();input.importProfile='lingmo-v7';
  if(mode==='unadopted')input.assets[0].design.state='UNADOPTED';
  if(mode==='dependency'){input.assets.push(asset('身份祖先'));input.assets[0].design.dependencies=[{assetId:'A0002',assetRevision:1,purpose:'身份依赖'}];}
  await assets.initializeRegistry(root,input);const request=imageRequest();
  if(mode==='future')Object.assign(request.uses[0],{episode:2,sceneId:'2-1'});
  if(mode==='registry')request.baseRevision=9;
  if(mode==='missing-source-use')request.uses[0].sceneId='1-unknown';
  if(mode==='dependency'){await assets.applyChange(root,{...change(),action:'revise',assetId:'A0002',asset:asset('身份祖先'),scopeUses:[use()]});request.baseRevision=2;}
  await assert.rejects(assets.createImageQueue(root,request),{code:{future:'SEASON_OUT_OF_SCOPE',unadopted:'ASSET_DESIGN_UNADOPTED',dependency:'ASSET_DESIGN_DEPENDENCY_STALE',registry:'ASSET_BASE_REVISION','missing-source-use':'SEASON_OUT_OF_SCOPE'}[mode]});
 }
});
test('image-queue真实CLI独占创建，不能覆盖清单',async t=>{
 const root=await sandbox(t),registry=join(root,'registry');await assets.initializeRegistry(registry,seed());
 const input=await save(root,'request.json',imageRequest()),output=join(root,'queue.json');
 let result=cli(['image-queue','--registry',registry,'--input',input,'--output',output]);assert.equal(result.status,0,result.stderr);
 result=cli(['image-queue','--registry',registry,'--input',input,'--output',output]);assert.notEqual(result.status,0);assert.match(result.stderr,/EEXIST/);
});
test('sync真实CLI传递冻结源锁，季库还必须提供同一范围',async t=>{
 const root=await sandbox(t),fixture=JSON.parse(await readFile(join(base,'tests/fixtures/export-bundle.json'),'utf8'));
 fixture.testFixture=false;fixture.sourceHash=sourceHash;
 const input=seed(),syncScope=scope();syncScope.episodes=structuredClone(fixture.scope);syncScope.readingScope=[...structuredClone(fixture.scope),{number:2,sceneIds:['2-1']}];syncScope.ending.sceneId=fixture.scope.at(-1).sceneIds.at(-1);input.productionScope=syncScope;input.assets=fixture.assets.map(a=>({...a,aliases:[],scopeUses:fixture.scope[0].sceneIds.map(sceneId=>({episode:1,sceneId})),origin:asset().origin,design:asset().design}));
 await assets.initializeRegistry(join(root,'registry'),input);const {registry}=await assets.readRegistry(join(root,'registry'));
 fixture.assets=fixture.assets.map((a,i)=>({...a,assetId:registry.assets[i].id,assetRevision:1}));fixture.assetRegistry={id:registry.id,revision:1};
 fixture.productionScope=syncScope;fixture.scopeHash=registry.productionScopeHash;fixture.generationTextVersion='voice-delivery-v1';
 const sourceLock={version:'series-source-lock-v1',sourceHash,episodes:fixture.episodes.map(({number,source})=>({number,source}))};
 fixture.sourceLockHash=createHash('sha256').update(JSON.stringify(sourceLock)).digest('hex');
 const bundle=await save(root,'bundle.json',fixture),lock=await save(root,'lock.json',sourceLock),scopePath=await save(root,'scope.json',syncScope),output=join(root,'receipt.json');
 let result=cli(['sync','--registry',join(root,'registry'),'--input',bundle,'--output',output]);assert.notEqual(result.status,0);
 result=cli(['sync','--registry',join(root,'registry'),'--input',bundle,'--output',output,'--source-lock',lock,'--season-scope',scopePath]);assert.equal(result.status,0,result.stderr);assert.equal(JSON.parse(await readFile(output,'utf8')).productionScopeHash,registry.productionScopeHash);
});
