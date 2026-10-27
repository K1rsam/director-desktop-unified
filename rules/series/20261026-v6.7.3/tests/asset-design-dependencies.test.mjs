import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,readFile,rm} from 'node:fs/promises';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import {applyChange,checkAssetBindings,initializeRegistry,readRegistry,validateRegistry} from '../scripts/assets.mjs';

const sourceHash='d'.repeat(64);
const dependency=(assetId,assetRevision=1)=>({assetId,assetRevision,purpose:'继承已采用的形体与材质设计'});
function asset(id,name,type='CH'){
 return {id,revision:1,name,key:name+type,type,status:'常态',aliases:[],origin:{kind:'source',sourceRefs:['E11:R1'],basis:'合成来源中的稳定形貌'},design:{state:'TEXT_ADOPTED',text:'保留稳定轮廓、材质和识别特征。'}};
}
function setup(){
 const assets=[asset('A0001','人物基础'),asset('A0002','人物战甲'),asset('A0003','人物战损'),asset('A0004','大厅','SC')];
 assets[1].design.dependencies=[dependency('A0001')];
 assets[2].design.dependencies=[dependency('A0002')];
 const registry={version:'series-assets-v1',id:'design-dependency-fixture',sourceHash,revision:1,assets,requests:[]};
 return {registry,bundle:bundleFor(registry)};
}
function bundleFor(registry,used=['A0003','A0004']){
 const assets=used.map(id=>registry.assets.find(a=>a.id===id)).map(({id,revision,key,name,type,status})=>({assetId:id,assetRevision:revision,key,name,type,status}));
 return {sourceHash,assetRegistry:{id:registry.id,revision:registry.revision},assets,episodes:[{number:11,draft:{units:[{id:'U7',sceneId:'11-2',bindings:assets.map(a=>a.key)}]}}]};
}
function expectDependencyError(run,code,details){
 assert.throws(run,error=>{
  assert.equal(error.code,code);
  for(const detail of ['第11集/11-2/U7',...details])assert.ok(error.message.includes(detail),`错误缺少 ${detail}：${error.message}`);
  return true;
 });
}
async function sandbox(t){
 const dir=await mkdtemp(join(tmpdir(),'asset-design-dependencies-'));
 t.after(()=>rm(dir,{recursive:true,force:true}));
 return join(dir,'assets');
}
async function revise(root,assetId,changes){
 const {registry}=await readRegistry(root),current=registry.assets.find(a=>a.id===assetId);
 return applyChange(root,{version:'series-asset-change-v1',baseRevision:registry.revision,requestId:`DESIGN-${registry.revision}-${assetId}`,stage:'asset-design',action:'revise',assetId,asset:{type:current.type,...changes},uses:[],reason:'按实际设计依赖复核后修订',duplicateReview:'已核同一资产，保留稳定身份'});
}

test('未声明设计依赖的旧库兼容，空依赖数组同样可用',()=>{
 const {registry}=setup();for(const a of registry.assets)delete a.design.dependencies;
 registry.assets[2].design.dependencies=[];
 assert.doesNotThrow(()=>validateRegistry(registry));
 assert.deepEqual(checkAssetBindings(bundleFor(registry),registry).assets,['A0003','A0004']);
});

const invalidDependencies=[
 ['不是数组',{}],['null数组',null],['空条目',[null]],['缺少用途',[{assetId:'A0001',assetRevision:1}]],
 ['空用途',[{...dependency('A0001'),purpose:' '}]],['未知ID',[dependency('A9999')]],
 ['零版本',[dependency('A0001',0)]],['小数版本',[dependency('A0001',1.5)]],['字符串版本',[dependency('A0001','1')]],
 ['未来版本',[dependency('A0001',2)]],['重复依赖',[dependency('A0001'),dependency('A0001')]],['自身依赖',[dependency('A0002')]]
];
for(const [label,dependencies] of invalidDependencies)test(`总库拒绝设计依赖：${label}`,()=>{
 const {registry}=setup();registry.assets[1].design.dependencies=dependencies;
 assert.throws(()=>validateRegistry(registry),{code:'ASSET_DESIGN_DEPENDENCY'});
});

test('总库保留旧依赖版本及待采用祖先，读取时不将待复核记录判坏',()=>{
 const {registry}=setup();registry.assets[0].revision=2;registry.assets[0].design.state='UNADOPTED';registry.revision=2;
 const before=structuredClone(registry);
 assert.doesNotThrow(()=>validateRegistry(registry));assert.deepEqual(registry,before);
});

test('总库拒绝跨三项设计循环，即使循环项未被本集使用',()=>{
 const {registry}=setup();registry.assets[0].design.dependencies=[dependency('A0003')];
 assert.throws(()=>validateRegistry(registry),error=>{
  assert.equal(error.code,'ASSET_DESIGN_DEPENDENCY_CYCLE');
  for(const id of ['A0001','A0002','A0003'])assert.ok(error.message.includes(id));
  return true;
 });
});

test('共享祖先的无环依赖可用，祖先不被自动加入镜头消费或图片清单',()=>{
 const {registry,bundle}=setup();registry.assets[2].design.dependencies.push(dependency('A0001'));
 registry.assets[0].design.state='REFERENCE_ADOPTED';
 const before=structuredClone(bundle),result=checkAssetBindings(bundle,registry);
 assert.deepEqual(result.assets,['A0003','A0004']);assert.equal(result.uses.length,2);assert.deepEqual(bundle,before);
});

test('直接设计依赖版本改变时拒绝，报当前坐标、子项及祖先版本',()=>{
 const {registry,bundle}=setup();registry.assets[1].revision=2;registry.revision=2;
 expectDependencyError(()=>checkAssetBindings(bundle,registry),'ASSET_DESIGN_DEPENDENCY_STALE',['A0003@1','A0002@1','A0002@2']);
});

test('祖先版本改变经多层递归拒绝，检查不自动升级或改写下游设计',()=>{
 const {registry,bundle}=setup();registry.assets[0].revision=2;registry.revision=2;
 const beforeRegistry=structuredClone(registry),beforeBundle=structuredClone(bundle);
 expectDependencyError(()=>checkAssetBindings(bundle,registry),'ASSET_DESIGN_DEPENDENCY_STALE',['A0003@1','A0002@1','A0001@1','A0001@2']);
 assert.deepEqual(registry,beforeRegistry);assert.deepEqual(bundle,beforeBundle);
});

for(const state of ['STALE','UNADOPTED'])test(`当前版本祖先的 ${state} 状态阻止实际消费者采用`,()=>{
 const {registry,bundle}=setup();registry.assets[0].design.state=state;
 expectDependencyError(()=>checkAssetBindings(bundle,registry),'ASSET_DESIGN_DEPENDENCY_UNADOPTED',['A0003@1','A0002@1','A0001@1',state]);
});

test('仅消费部分素材时，无关新增、未采用卡和未用的过期链不阻塞独立设计',()=>{
 const {registry,bundle}=setup(),independent=bundleFor(registry,['A0004']);
 registry.assets[0].revision=2;registry.revision=3;
 const unused=asset('A0005','路人');unused.design.state='UNADOPTED';registry.assets.push(unused);
 const unusedDependent=asset('A0006','旧设计候选');unusedDependent.design.dependencies=[dependency('A0001')];registry.assets.push(unusedDependent);
 assert.deepEqual(checkAssetBindings(independent,registry).assets,['A0004']);
 assert.throws(()=>checkAssetBindings(bundle,registry),{code:'ASSET_DESIGN_DEPENDENCY_STALE'});
});

test('parentId只表达身份关系，不自动形成设计依赖或阻塞独立子形象',()=>{
 const {registry,bundle}=setup();registry.assets[0].revision=2;registry.assets[0].design.state='STALE';registry.revision=2;
 registry.assets[1].parentId='A0001';delete registry.assets[1].design.dependencies;
 assert.doesNotThrow(()=>checkAssetBindings(bundle,registry));
 assert.equal(registry.assets[1].design.dependencies,undefined);
});

test('直接使用资产的旧版本错误也包含当前集场单元及双方版本',()=>{
 const {registry,bundle}=setup();registry.assets[2].revision=2;registry.revision=2;
 expectDependencyError(()=>checkAssetBindings(bundle,registry),'ASSET_REGISTRY_STALE',['A0003@1','A0003@2']);
});

test('真实init和逐层revise保留旧依赖，复核完成并更新消费者版本后恢复',async t=>{
 const {registry}=setup(),root=await sandbox(t);
 await initializeRegistry(root,{sourceHash,assets:registry.assets});
 const first=await readRegistry(root),oldBytes=await readFile(first.file,'utf8'),oldBundle=bundleFor(first.registry);
 await revise(root,'A0001',{design:{...first.registry.assets[0].design,text:'重新采用的基础材质。'}});
 let current=(await readRegistry(root)).registry;
 assert.equal(current.assets[1].revision,1);assert.equal(current.assets[2].revision,1);
 assert.deepEqual(current.assets[1].design.dependencies,[dependency('A0001')]);
 assert.doesNotThrow(()=>validateRegistry(current));
 expectDependencyError(()=>checkAssetBindings(oldBundle,current),'ASSET_DESIGN_DEPENDENCY_STALE',['A0003@1','A0001@1','A0001@2']);
 assert.deepEqual(checkAssetBindings(bundleFor(first.registry,['A0004']),current).assets,['A0004']);
 await revise(root,'A0002',{design:{...current.assets[1].design,dependencies:[dependency('A0001',2)]}});
 current=(await readRegistry(root)).registry;
 expectDependencyError(()=>checkAssetBindings(oldBundle,current),'ASSET_DESIGN_DEPENDENCY_STALE',['A0003@1','A0002@1','A0002@2']);
 await revise(root,'A0003',{design:{...current.assets[2].design,dependencies:[dependency('A0002',2)]}});
 current=(await readRegistry(root)).registry;
 assert.throws(()=>checkAssetBindings(oldBundle,current),{code:'ASSET_REGISTRY_STALE'});
 assert.deepEqual(checkAssetBindings(bundleFor(current),current).assets,['A0003','A0004']);
 assert.equal(await readFile(first.file,'utf8'),oldBytes);
});

test('循环依赖的revise被拒绝，不发布新快照或破坏当前可读总库',async t=>{
 const {registry}=setup(),root=await sandbox(t);
 await initializeRegistry(root,{sourceHash,assets:registry.assets});
 const first=await readRegistry(root),oldBytes=await readFile(first.file,'utf8');
 await assert.rejects(revise(root,'A0001',{design:{...first.registry.assets[0].design,dependencies:[dependency('A0003')]}}),{code:'ASSET_DESIGN_DEPENDENCY_CYCLE'});
 assert.equal((await readRegistry(root)).registry.revision,1);assert.equal(await readFile(first.file,'utf8'),oldBytes);
 await assert.rejects(readFile(join(root,'v000002.json')),{code:'ENOENT'});
});

test('完整design.full及未知扩展字段经init和正常revise原样保持',async t=>{
 const {registry}=setup(),root=await sandbox(t);
 const full={version:'full-design-fixture-v1',appearance:{silhouette:'肩宽与头身比例固定',costume:{layers:['内衫','外甲'],material:'旧铜'}},space:{landmarks:['左门','后窗'],passage:'左门通往后廊'},productionNotes:{views:['正面','侧面','背面']}};
 registry.assets[0].design.full=full;registry.assets[0].design.extension={futureField:['保留','原值']};
 await initializeRegistry(root,{sourceHash,assets:registry.assets});
 let current=(await readRegistry(root)).registry;
 assert.deepEqual(current.assets[0].design.full,full);
 await revise(root,'A0001',{status:'正常使用状态'});
 current=(await readRegistry(root)).registry;
 assert.deepEqual(current.assets[0].design.full,full);assert.deepEqual(current.assets[0].design.extension,{futureField:['保留','原值']});
 const fullDesign={...current.assets[0].design,text:'更新短生产卡，完整设计继续保留。'};
 await revise(root,'A0001',{design:fullDesign});
 current=(await readRegistry(root)).registry;
 assert.deepEqual(current.assets[0].design,fullDesign);assert.deepEqual(current.assets[0].design.full,full);
});
