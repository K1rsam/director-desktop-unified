import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync, writeFileSync, mkdtempSync, rmSync, existsSync, mkdirSync} from 'node:fs';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {checkBundle} from '../scripts/storyboard.mjs';
import {checkAssetBindings,validateRegistry} from '../scripts/assets.mjs';

const base=fileURLToPath(new URL('..',import.meta.url));
const fixture=JSON.parse(readFileSync(join(base,'tests/fixtures/export-bundle.json'),'utf8'));
const sourceHash='a'.repeat(64);
function setup(){
 const bundle=structuredClone(fixture);bundle.testFixture=false;bundle.sourceHash=sourceHash;
 const assets=bundle.assets.map((a,i)=>({...a,id:`A${String(i+1).padStart(4,'0')}`,revision:1,aliases:[],origin:{kind:'source',sourceRefs:['E01:R1'],basis:'合成来源的文字资产基线'},design:{state:'TEXT_ADOPTED',text:'沿用合成样例的源特征与最小文字设计。'}}));
 bundle.assets=bundle.assets.map((a,i)=>({...a,assetId:assets[i].id,assetRevision:1}));
 const registry={version:'series-assets-v1',id:'fixture-registry',sourceHash,revision:1,assets,requests:[]};
 bundle.assetRegistry={id:registry.id,revision:1};
 return {bundle,registry};
}
const phone=()=>({key:'直播手机PR',name:'直播手机',type:'PR',status:'常态',aliases:['手持拍摄手机'],origin:{kind:'design',sourceRefs:['E01:R1'],basis:'原文直播仅支持拍摄需求；按已确认方向选择手机。'},design:{state:'TEXT_ADOPTED',text:'黑色直板手机，背部双摄，保持同一设备。'}});
function addPhone(bundle,assetId='A0005',assetRevision=1){
 const p=phone();bundle.assets.push({key:p.key,name:p.name,type:p.type,status:p.status,assetId,assetRevision});
 const e=bundle.episodes[0],u=e.draft.units[0];u.props.push(p.name);u.bindings.push(p.key);e.scenes[0].bindings.push(p.key);u.shots[0].picture+='左手举着直播手机，镜头朝向门厅。';
}
test('生产输入不得缺少统一总库；合成格式样例仍可独立使用',()=>{
 const {bundle}=setup();assert.throws(()=>checkBundle(bundle),{code:'ASSET_REGISTRY_REQUIRED'});assert.equal(checkBundle(fixture).ok,true);
});
test('阶段三自行补进bundle的直播手机，未回补总库时拒绝',()=>{
 const {bundle,registry}=setup();addPhone(bundle);assert.throws(()=>checkBundle(bundle,{registry}),{code:'ASSET_REGISTRY_MISSING'});
});
test('总库和文字卡采用后，实际绑定手机才能关闭补项',()=>{
 const {bundle,registry}=setup();const p={...phone(),id:'A0005',revision:1};registry.assets.push(p);registry.revision=2;
 registry.requests.push({id:'R-phone',assetId:p.id,assetRevision:1,uses:[{episode:1,sceneId:'1-1',unitId:'U1'}]});
 assert.throws(()=>checkBundle(bundle,{registry}),{code:'ASSET_REQUEST_UNCONSUMED'});
 addPhone(bundle);const result=checkBundle(bundle,{registry});assert.deepEqual(result.assetRegistry.closedRequests,['R-phone']);
});
test('已用资产版本改变会拦截，新增无关资产不使旧采用稿失效',()=>{
 const {bundle,registry}=setup();registry.assets.push({...phone(),id:'A0005',revision:1});registry.revision=2;
 assert.equal(checkBundle(bundle,{registry}).ok,true);
 registry.assets[0].revision=2;assert.throws(()=>checkBundle(bundle,{registry}),{code:'ASSET_REGISTRY_STALE'});
});
test('同ID也不能偷换正式名、状态或未采用设计',()=>{
 const {bundle,registry}=setup();bundle.assets[0].status='另一状态';assert.throws(()=>checkBundle(bundle,{registry}),{code:'ASSET_REGISTRY_MISMATCH'});
 bundle.assets[0].status=registry.assets[0].status;registry.assets[0].design.state='UNADOPTED';assert.throws(()=>checkBundle(bundle,{registry}),{code:'ASSET_DESIGN_UNADOPTED'});
});
test('其它剧本、其它总库和未来总库版本不能混用',()=>{
 for(const change of [b=>b.sourceHash='b'.repeat(64),b=>b.assetRegistry.id='other',b=>b.assetRegistry.revision=99]){
  const {bundle,registry}=setup();change(bundle);assert.throws(()=>checkBundle(bundle,{registry}),{code:'ASSET_REGISTRY_BINDING'});
 }
});
test('必要持续状态允许共享原始源名，但仍保留不同ID和正式状态名',()=>{
 const {bundle,registry}=setup(),original=registry.assets[0];original.originalName=original.name;
 registry.assets.push({...structuredClone(original),id:'A0005',parentId:original.id,name:original.name+'·破损',key:original.name+'·破损'+original.type,status:'持续破损'});registry.revision=2;
 assert.equal(checkBundle(bundle,{registry}).ok,true);
});
test('残留生产绑定时，测试标记不能降级绕过总库',()=>{
 for(const retain of ['sourceHash','assetId','assetRevision']){
  const {bundle}=setup();bundle.testFixture=true;delete bundle.assetRegistry;if(retain!=='sourceHash')delete bundle.sourceHash;
  for(const a of bundle.assets){if(retain!=='assetId')delete a.assetId;if(retain!=='assetRevision')delete a.assetRevision;}
  assert.throws(()=>checkBundle(bundle),{code:'ASSET_REGISTRY_REQUIRED'});
 }
});
const cli=(script,args)=>spawnSync(process.execPath,[join(base,'scripts',script),...args],{encoding:'utf8'});
const save=(dir,name,value)=>{const p=join(dir,name);writeFileSync(p,JSON.stringify(value,null,2));return p;};
const sandbox=fn=>{const dir=mkdtempSync(join(tmpdir(),'asset-sync-'));try{fn(dir);}finally{rmSync(dir,{recursive:true,force:true});}};
test('真实CLI回补、去重、旧版本保留、消费回执和三步导出',()=>sandbox(dir=>{
 const {bundle,registry}=setup(),root=join(dir,'共享资产');
 const seed=save(dir,'seed.json',{sourceHash,assets:registry.assets.map(({id,revision,...a})=>a)});
 let r=cli('assets.mjs',['init','--registry',root,'--input',seed]);assert.equal(r.status,0,r.stderr);
 const initial=JSON.parse(r.stdout);bundle.assetRegistry={id:initial.id,revision:initial.revision};
 const old=readFileSync(join(root,'v000001.json'),'utf8');
 const proposal={version:'series-asset-change-v1',baseRevision:1,requestId:'R-phone',action:'add',reason:'第三阶段发现缺少直播拍摄设备',duplicateReview:'现有目录无此设备；手持拍摄手机为拟登记别名',uses:[{episode:1,sceneId:'1-1',unitId:'U1'}],asset:phone()};
 const request=save(dir,'request.json',proposal);
 r=cli('assets.mjs',['apply','--registry',root,'--input',request]);assert.equal(r.status,0,r.stderr);const added=JSON.parse(r.stdout);
 assert.equal(readFileSync(join(root,'v000001.json'),'utf8'),old);
 assert.match(cli('assets.mjs',['apply','--registry',root,'--input',request]).stderr,/ASSET_BASE_REVISION/);
 proposal.baseRevision=2;proposal.requestId='R-duplicate';proposal.asset={...phone(),name:'手持拍摄手机',key:'手持拍摄手机PR',aliases:[]};
 r=cli('assets.mjs',['apply','--registry',root,'--input',save(dir,'duplicate.json',proposal)]);assert.notEqual(r.status,0);assert.match(r.stderr,/ASSET_ALIAS_CONFLICT/);
 const bundlePath=save(dir,'bundle.json',bundle),out=join(dir,'结果.md'),receipt=join(dir,'使用回执.json');
 r=cli('storyboard.mjs',['check','--input',bundlePath,'--registry',root]);assert.notEqual(r.status,0);assert.match(r.stderr,/ASSET_REQUEST_UNCONSUMED/);
 addPhone(bundle,added.asset.id,added.asset.revision);save(dir,'bundle.json',bundle);
 r=cli('assets.mjs',['sync','--registry',root,'--input',bundlePath,'--output',receipt]);assert.equal(r.status,0,r.stderr);
 assert.deepEqual(JSON.parse(readFileSync(receipt,'utf8')).closedRequests,['R-phone']);
 for(const command of ['check','export','verify']){r=cli('storyboard.mjs',[command,'--input',bundlePath,'--registry',root,...(command==='check'?[]:['--output',out])]);assert.equal(r.status,0,r.stderr);}
 assert.match(readFileSync(out,'utf8'),/直播手机PR/);assert.doesNotMatch(readFileSync(out,'utf8'),/fixture-registry|assetRevision|R-phone/);
 assert.equal(existsSync(join(root,'.lock')),false);
}));
test('阶段二在尚无单元时可采用文字卡；阶段三补项仍须实际消费坐标',()=>sandbox(dir=>{
 const {registry}=setup(),root=join(dir,'assets');
 const seed=save(dir,'seed.json',{sourceHash,assets:registry.assets.map(({id,revision,...a})=>({...a,design:{state:'UNADOPTED',text:''}}))});
 assert.equal(cli('assets.mjs',['init','--registry',root,'--input',seed]).status,0);
 const asset=registry.assets[0];
 const request={version:'series-asset-change-v1',baseRevision:1,requestId:'DESIGN-1',stage:'asset-design',action:'revise',assetId:asset.id,asset,reason:'阶段二采用最小文字卡',duplicateReview:'已有同一资产ID，沿用身份',uses:[]};
 let r=cli('assets.mjs',['apply','--registry',root,'--input',save(dir,'design.json',request)]);assert.equal(r.status,0,r.stderr);
 const loaded=JSON.parse(cli('assets.mjs',['status','--registry',root]).stdout).registry;
 assert.equal(loaded.assets[0].design.state,'TEXT_ADOPTED');assert.equal(loaded.requests.length,0);
 request.baseRevision=2;request.requestId='PRODUCTION-1';request.stage='production';
 r=cli('assets.mjs',['apply','--registry',root,'--input',save(dir,'production.json',request)]);assert.notEqual(r.status,0);assert.match(r.stderr,/ASSET_REQUEST/);
}));
test('整编移位或撤去可选设计时，可修订使用请求且保留历史资产',()=>sandbox(dir=>{
 const {registry}=setup(),root=join(dir,'assets'),seed=save(dir,'seed.json',{sourceHash,assets:registry.assets.map(({id,revision,...a})=>a)});
 assert.equal(cli('assets.mjs',['init','--registry',root,'--input',seed]).status,0);
 const change={version:'series-asset-change-v1',baseRevision:1,requestId:'MOVE-1',action:'add',asset:phone(),reason:'有据设计',duplicateReview:'无同项',uses:[{episode:1,sceneId:'1-1',unitId:'U1'}]};
 assert.equal(cli('assets.mjs',['apply','--registry',root,'--input',save(dir,'add.json',change)]).status,0);
 const amend={version:'series-asset-amend-v1',baseRevision:2,requestId:'MOVE-1',action:'retarget',uses:[{episode:1,sceneId:'1-1',unitId:'U2'}],reason:'整编后单元重新编号',sourceReview:'保持原动作和设备，仅更新坐标'};
 let r=cli('assets.mjs',['amend','--registry',root,'--input',save(dir,'amend.json',amend)]);assert.equal(r.status,0,r.stderr);
 amend.baseRevision=3;amend.action='withdraw';amend.uses=[];amend.reason='有据替换为已登记固定机位';amend.sourceReview='原文只写直播，未指定手机；拍摄功能仍保留';
 r=cli('assets.mjs',['amend','--registry',root,'--input',save(dir,'withdraw.json',amend)]);assert.equal(r.status,0,r.stderr);
 const now=JSON.parse(cli('assets.mjs',['status','--registry',root]).stdout).registry;
 assert.equal(now.assets.length,registry.assets.length+1);assert.equal(now.requests[0].state,'withdrawn');
 assert.equal(JSON.parse(readFileSync(join(root,'v000002.json'),'utf8')).requests[0].uses[0].unitId,'U1');
}));
test('同物复用不造新资产或抬升资产版本；跨范围请求不提前全部关闭',()=>sandbox(dir=>{
 const {bundle,registry}=setup(),root=join(dir,'assets'),seed=save(dir,'seed.json',{sourceHash,assets:registry.assets.map(({id,revision,...a})=>a)});
 const initialized=JSON.parse(cli('assets.mjs',['init','--registry',root,'--input',seed]).stdout);bundle.assetRegistry.id=initialized.id;
 const key=registry.assets.find(a=>a.type==='PR');
 const request={version:'series-asset-change-v1',baseRevision:1,requestId:'REUSE-1',action:'reuse',assetId:key.id,reason:'两路确认是同一物件',duplicateReview:'同持有关系和状态，只使用不同称呼',uses:[{episode:1,sceneId:'1-1',unitId:'U1'},{episode:2,sceneId:'2-1',unitId:'U2'}]};
 let r=cli('assets.mjs',['apply','--registry',root,'--input',save(dir,'reuse.json',request)]);assert.equal(r.status,0,r.stderr);assert.equal(JSON.parse(r.stdout).asset.revision,1);
 const loaded=JSON.parse(cli('assets.mjs',['status','--registry',root]).stdout).registry;assert.equal(loaded.assets.length,registry.assets.length);
 const result=checkBundle(bundle,{registry:loaded}).assetRegistry;assert.deepEqual(result.closedRequests,[]);assert.deepEqual(result.pendingOutsideScope,['REUSE-1']);
}));
test('总库原地篡改和遗留写锁不会被静默接受或重置',()=>sandbox(dir=>{
 const {registry}=setup(),root=join(dir,'assets'),seed=save(dir,'seed.json',{sourceHash,assets:registry.assets.map(({id,revision,...a})=>a)});
 assert.equal(cli('assets.mjs',['init','--registry',root,'--input',seed]).status,0);
 assert.match(cli('assets.mjs',['init','--registry',root,'--input',seed]).stderr,/ASSET_EXISTS/);
 const snap=join(root,'v000001.json');writeFileSync(snap,readFileSync(snap,'utf8')+' ');
 assert.match(cli('assets.mjs',['status','--registry',root]).stderr,/ASSET_SNAPSHOT_CHANGED/);
 mkdirSync(join(root,'.lock'));
 assert.match(cli('assets.mjs',['apply','--registry',root,'--input',seed]).stderr,/ASSET_LOCKED/);assert.equal(existsSync(join(root,'.lock')),true);
}));
test('补文字卡时省略旧来源及家族字段，不得丢失这些上游数据',()=>sandbox(dir=>{
 const {registry}=setup(),root=join(dir,'assets');registry.assets[0].upstreamKey='LOC_001';registry.assets[0].sourceFacts={text:'原文明示门厅',refs:['E01:R1']};
 const seed=save(dir,'seed.json',{sourceHash,assets:registry.assets.map(({id,revision,...a})=>a)});assert.equal(cli('assets.mjs',['init','--registry',root,'--input',seed]).status,0);
 const {upstreamKey,sourceFacts,...asset}=registry.assets[0];asset.design={state:'TEXT_ADOPTED',text:'保留门厅空间，补足浅灰墙面识别基准。'};
 const request={version:'series-asset-change-v1',baseRevision:1,requestId:'DESIGN-METADATA',stage:'asset-design',action:'revise',assetId:asset.id,asset,reason:'补足文字设计',duplicateReview:'复用原地点',uses:[]};
 const r=cli('assets.mjs',['apply','--registry',root,'--input',save(dir,'change.json',request)]);assert.equal(r.status,0,r.stderr);
 const current=JSON.parse(cli('assets.mjs',['status','--registry',root]).stdout).registry.assets[0];assert.equal(current.upstreamKey,upstreamKey);assert.deepEqual(current.sourceFacts,sourceFacts);
}));

const v7Setup=()=>{
 const state=setup();state.registry.importProfile='lingmo-v7';state.bundle.exportProfile='lingmo-v7';
 state.registry.assets.forEach((a,i)=>{a.importName=a.name.replaceAll('·','');a.placeholderOrdinal=i;Object.assign(state.bundle.assets[i],{importName:a.importName,placeholderOrdinal:i});});
 for(const e of state.bundle.episodes)for(const u of e.draft.units)u.imageBindings=[...u.bindings];
 return state;
};
const initV7=(dir,assets)=>{
 const root=join(dir,'assets');const r=cli('assets.mjs',['init','--registry',root,'--input',save(dir,'seed.json',{sourceHash,importProfile:'lingmo-v7',assets})]);
 return {root,r};
};
test('v7安全导入名独立分配，Python isalnum语义保留数值字符并移除组合符与结构符',()=>sandbox(dir=>{
 const original='直播｜手机、（Ⅰ²⅓）A\u0301\u{1E4D0}',asset={...phone(),name:original,key:original+'PR',originalName:'直播拍摄设备',familyId:'设备1'};
 const {root,r}=initV7(dir,[asset]);assert.equal(r.status,0,r.stderr);
 const current=JSON.parse(cli('assets.mjs',['status','--registry',root]).stdout).registry;
 const expected=spawnSync('python3',['-c','import json,sys; print(json.dumps("".join(c for c in json.load(sys.stdin) if c.isalnum())))'],{input:JSON.stringify(original),encoding:'utf8'});assert.equal(expected.status,0,expected.stderr);
 assert.equal(current.importProfile,'lingmo-v7');assert.equal(current.assets[0].importName,JSON.parse(expected.stdout));assert.equal(current.assets[0].placeholderOrdinal,0);
 assert.equal(current.assets[0].name,original);assert.equal(current.assets[0].key,original+'PR');assert.equal(current.assets[0].originalName,'直播拍摄设备');assert.equal(current.assets[0].familyId,'设备1');assert.deepEqual(current.assets[0].design,asset.design);
}));
test('v7拒绝清洗后同类重名、大小写与文件名碰撞，不能靠ID合并',()=>{
 for(const names of [['手机（左）','手机左'],['Phone-A','phoneA'],['麦克风Ｓ','麦克风S']])sandbox(dir=>{
  const assets=names.map(n=>({...phone(),name:n,key:n+'PR',aliases:[]}));
  const {root,r}=initV7(dir,assets);assert.notEqual(r.status,0);assert.match(r.stderr,/ASSET_IMPORT_COLLISION/);assert.equal(existsSync(join(root,'current.json')),false);
 });
});
test('v7拒绝空名、无、非法显式安全名与错误占位编号',()=>{
 for(const asset of [{...phone(),name:'（）',key:'（）PR'},{...phone(),name:'（无）',key:'（无）PR'},{...phone(),importName:'手机/图'},{...phone(),placeholderOrdinal:-1}])sandbox(dir=>{
  const {r}=initV7(dir,[asset]);assert.notEqual(r.status,0);assert.match(r.stderr,/ASSET_IMPORT_NAME|ASSET_PLACEHOLDER_ORDINAL/);
 });
 const {registry}=v7Setup();
 for(const change of [r=>r.assets[1].placeholderOrdinal=0,r=>r.assets[1].placeholderOrdinal=5701,r=>r.assets[1].placeholderOrdinal=1.2]){
  const changed=structuredClone(registry);change(changed);assert.throws(()=>validateRegistry(changed),{code:'ASSET_PLACEHOLDER_ORDINAL'});
 }
});
test('v7新增仅追加编号，改文字卡保持导入名和编号，不能重分配旧编号',()=>sandbox(dir=>{
 const {registry}=setup(),{root,r}=initV7(dir,registry.assets);assert.equal(r.status,0,r.stderr);
 const first=readFileSync(join(root,'v000001.json'),'utf8');
 const request={version:'series-asset-change-v1',baseRevision:1,requestId:'V7-ADD',stage:'asset-design',action:'add',asset:phone(),reason:'阶段二补设备',duplicateReview:'已核无同项',uses:[]};
 let out=cli('assets.mjs',['apply','--registry',root,'--input',save(dir,'add.json',request)]);assert.equal(out.status,0,out.stderr);
 let current=JSON.parse(cli('assets.mjs',['status','--registry',root]).stdout).registry;assert.equal(current.assets.at(-1).placeholderOrdinal,4);
 request.baseRevision=2;request.requestId='V7-REVISE';request.action='revise';request.assetId=current.assets[0].id;request.asset={type:current.assets[0].type,design:{state:'TEXT_ADOPTED',text:'补光后面罩边缘可辨，身份保持。'}};
 out=cli('assets.mjs',['apply','--registry',root,'--input',save(dir,'revise.json',request)]);assert.equal(out.status,0,out.stderr);
 current=JSON.parse(cli('assets.mjs',['status','--registry',root]).stdout).registry;
 assert.equal(current.assets[0].placeholderOrdinal,0);assert.equal(current.assets[0].importName,'值守者面罩');assert.equal(current.assets[0].revision,2);assert.equal(readFileSync(join(root,'v000001.json'),'utf8'),first);
 request.baseRevision=3;request.requestId='V7-RENUMBER';request.asset.placeholderOrdinal=20;
 out=cli('assets.mjs',['apply','--registry',root,'--input',save(dir,'renumber.json',request)]);assert.notEqual(out.status,0);assert.match(out.stderr,/ASSET_PLACEHOLDER_ORDINAL/);
}));
test('旧库prepare-import只新增映射快照，原资产版本及采用状态保留',()=>sandbox(dir=>{
 const {bundle,registry}=setup(),root=join(dir,'assets'),seed=save(dir,'seed.json',{sourceHash,assets:registry.assets});
 assert.equal(cli('assets.mjs',['init','--registry',root,'--input',seed]).status,0);
 const old=readFileSync(join(root,'v000001.json'),'utf8'),input={version:'series-asset-import-v1',baseRevision:1,importProfile:'lingmo-v7',names:[{id:'A0001',importName:'值守者面罩状态'}]};
 const r=cli('assets.mjs',['prepare-import','--registry',root,'--input',save(dir,'prepare.json',input)]);assert.equal(r.status,0,r.stderr);
 const now=JSON.parse(cli('assets.mjs',['status','--registry',root]).stdout).registry;assert.equal(now.revision,2);assert.equal(now.importProfile,'lingmo-v7');assert.equal(now.assets[0].importName,'值守者面罩状态');assert.equal(now.assets[0].placeholderOrdinal,0);
 assert.deepEqual(now.assets.map(a=>a.revision),registry.assets.map(a=>a.revision));assert.deepEqual(now.assets.map(a=>a.design),registry.assets.map(a=>a.design));assert.equal(readFileSync(join(root,'v000001.json'),'utf8'),old);
 bundle.assetRegistry.id=now.id;assert.equal(checkBundle(bundle,{registry:now}).ok,true);
}));
test('v7生产绑定同时核当前导入映射，纯逻辑项保留但未用项不强行消费',()=>{
 const {bundle,registry}=v7Setup();assert.equal(checkAssetBindings(bundle,registry).assets.length,4);
 for(const field of ['importName','placeholderOrdinal']){
  const changed=structuredClone(bundle);changed.assets[0][field]=field==='importName'?'另一名字':30;
  assert.throws(()=>checkAssetBindings(changed,registry),{code:'ASSET_REGISTRY_MISMATCH'});
 }
 const unused={...phone(),id:'A0005',revision:1,importName:'直播手机',placeholderOrdinal:4};registry.assets.push(unused);registry.revision=2;
 assert.equal(checkAssetBindings(bundle,registry).assets.length,4);
 const last=bundle.episodes[0].draft.units.at(-1);last.imageBindings=[];
 assert.ok(checkAssetBindings(bundle,registry).uses.some(u=>u.unitId===last.id));
});
test('旧库迁移冲突失败不写新快照，明确给安全名后才可保留两个独立状态',()=>sandbox(dir=>{
 const assets=['手机（左）','手机左'].map((n,i)=>({...phone(),name:n,key:n+'PR',aliases:[],status:'状态'+i})),root=join(dir,'assets');
 assert.equal(cli('assets.mjs',['init','--registry',root,'--input',save(dir,'seed.json',{sourceHash,assets})]).status,0);
 const old=readFileSync(join(root,'v000001.json'),'utf8'),input={version:'series-asset-import-v1',baseRevision:1,importProfile:'lingmo-v7'};
 let r=cli('assets.mjs',['prepare-import','--registry',root,'--input',save(dir,'prepare.json',input)]);assert.notEqual(r.status,0);assert.match(r.stderr,/ASSET_IMPORT_COLLISION/);assert.equal(existsSync(join(root,'v000002.json')),false);
 input.names=[{id:'A0002',importName:'手机左侧状态'}];r=cli('assets.mjs',['prepare-import','--registry',root,'--input',save(dir,'resolved.json',input)]);assert.equal(r.status,0,r.stderr);
 const now=JSON.parse(cli('assets.mjs',['status','--registry',root]).stdout).registry;assert.deepEqual(now.assets.map(a=>a.name),assets.map(a=>a.name));assert.deepEqual(now.assets.map(a=>a.status),['状态0','状态1']);assert.equal(readFileSync(join(root,'v000001.json'),'utf8'),old);
 input.baseRevision=2;r=cli('assets.mjs',['prepare-import','--registry',root,'--input',save(dir,'repeat.json',input)]);assert.notEqual(r.status,0);assert.match(r.stderr,/ASSET_IMPORT_PROFILE/);
}));
test('v7修订导入名会增加单项版本，旧消费者失效但占位编号仍固定',()=>sandbox(dir=>{
 const {bundle,registry}=v7Setup(),{root,r}=initV7(dir,registry.assets);assert.equal(r.status,0,r.stderr);bundle.assetRegistry.id=JSON.parse(r.stdout).id;
 const request={version:'series-asset-change-v1',baseRevision:1,requestId:'RENAME-IMPORT',stage:'asset-design',action:'revise',assetId:'A0001',asset:{type:'CH',importName:'值守者面罩新版'},reason:'明确新的导入名',duplicateReview:'稳定身份不变，仅更名',uses:[]};
 const out=cli('assets.mjs',['apply','--registry',root,'--input',save(dir,'revise.json',request)]);assert.equal(out.status,0,out.stderr);
 const now=JSON.parse(cli('assets.mjs',['status','--registry',root]).stdout).registry;assert.equal(now.assets[0].revision,2);assert.equal(now.assets[0].placeholderOrdinal,0);assert.equal(now.assets[0].name,registry.assets[0].name);
 assert.throws(()=>checkAssetBindings(bundle,now),{code:'ASSET_REGISTRY_STALE'});
 bundle.assets[0].assetRevision=2;assert.throws(()=>checkAssetBindings(bundle,now),{code:'ASSET_REGISTRY_MISMATCH'});
 bundle.assets[0].importName=now.assets[0].importName;assert.equal(checkAssetBindings(bundle,now).assets.length,4);
}));
test('v7导入名不能注入标签、换行或路径，最高合法占位编号为5700',()=>{
 const {registry}=v7Setup();
 for(const value of ['守卫\n[场景] 假场','守卫@[别项CH]','../守卫','守卫\\场','\0守卫']){
  const altered=structuredClone(registry);altered.assets[0].importName=value;assert.throws(()=>validateRegistry(altered),{code:'ASSET_IMPORT_NAME'});
 }
 registry.assets[0].placeholderOrdinal=5700;assert.doesNotThrow(()=>validateRegistry(registry));
 const {bundle,registry:old}=setup();bundle.exportProfile='lingmo-v7';assert.throws(()=>checkAssetBindings(bundle,old),{code:'ASSET_IMPORT_PROFILE'});
});
