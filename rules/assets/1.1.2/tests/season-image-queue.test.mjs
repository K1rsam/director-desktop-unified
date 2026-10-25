import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm,readFile,writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {initializeRegistry,createImageQueue,applyChange} from '../../series-storyboard-director/scripts/assets.mjs';
const base=fileURLToPath(new URL('..',import.meta.url)),script=join(base,'scripts/validate-season-image-queue.mjs');
const stable=v=>JSON.stringify(v,(_,x)=>x&&typeof x==='object'&&!Array.isArray(x)?Object.fromEntries(Object.keys(x).sort().map(k=>[k,x[k]])):x);
const hash=v=>createHash('sha256').update(v).digest('hex');
async function setup(t){
 const root=await mkdtemp(join(tmpdir(),'season-image-'));t.after(()=>rm(root,{recursive:true,force:true}));
 const source=join(root,'source.txt'),sourceBytes=Buffer.from('第一集 合成身份出场。第二集 后续形态。');await writeFile(source,sourceBytes);const sourceHash=hash(sourceBytes);
 const productionScope={version:'series-season-scope-v1',id:'season-1',revision:1,seasonNumber:1,status:'locked',sourceHash,readingScope:[{number:1,sceneIds:['1-1']},{number:2,sceneIds:['2-1']}],episodes:[{number:1,sceneIds:['1-1']}],target:{minSeconds:7200,maxSeconds:9000,planningSeconds:8100},ending:{episode:1,sceneId:'1-1',sourceRef:'source.txt:L1',closure:'本季冲突闭合',hook:'保留后续悬念',estimate:{minSeconds:7200,maxSeconds:9000}},confirmation:{kind:'delegated',basis:'合成测试委托'}};
 const asset={name:'当前形态',key:'当前形态CH',type:'CH',status:'常态',scopeUses:[{episode:1,sceneId:'1-1'}],aliases:[],origin:{kind:'source',sourceRefs:['source.txt:L1'],basis:'本季出场'},design:{state:'TEXT_ADOPTED',text:'本季常态完整设计'}};
 const registry=join(root,'registry');await initializeRegistry(registry,{sourceHash,productionScope,assets:[asset]});
 const queue=await createImageQueue(registry,{version:'series-image-request-v1',baseRevision:1,uses:[{assetId:'A0001',assetRevision:1,episode:1,sceneId:'1-1',purpose:'本季身份板'}]});
 const item=queue.items[0],batch={version:'series-image-batch-v1',queueHash:queue.queueHash,sourceHash,productionScopeHash:queue.productionScopeHash,tasks:[Object.fromEntries(['itemId','taskId','assetId','assetRevision','designHash','filename','uses'].map(k=>[k,structuredClone(item[k])]))]};
 return {root,source,registry,queue,batch,asset};
}
async function run(s){const queue=join(s.root,'queue.json'),batch=join(s.root,'batch.json');await writeFile(queue,JSON.stringify(s.queue));await writeFile(batch,JSON.stringify(s.batch));return spawnSync(process.execPath,[script,'--queue',queue,'--batch',batch,'--registry',s.registry,'--source',s.source],{encoding:'utf8'});}
test('独立生图Skill真实CLI校验当前季候选批次，返回只读通过',async t=>{const s=await setup(t),r=await run(s);assert.equal(r.status,0,r.stderr);assert.equal(JSON.parse(r.stdout).tasks,1);assert.equal(JSON.parse(r.stdout).imageApproval,false);});
for(const [name,mutate] of [
 ['未来用途',s=>s.batch.tasks[0].uses[0].episode=2],
 ['未知任务',s=>s.batch.tasks[0].taskId='IMG-other'],
 ['设计旧版本',s=>s.batch.tasks[0].assetRevision=0],
 ['偷换文件名',s=>s.batch.tasks[0].filename='角色-未来形态.png'],
 ['删季范围',s=>{delete s.queue.productionScope;delete s.queue.productionScopeHash;const {queueHash,...payload}=s.queue;s.queue.queueHash=hash(stable(payload));s.batch.queueHash=s.queue.queueHash;}],
 ['改队列内容不更新hash',s=>s.queue.items[0].design.text='未来服装'],
 ['缺批次范围hash',s=>delete s.batch.productionScopeHash]
])test(`批次派发前拒绝${name}`,async t=>{const s=await setup(t);mutate(s);const r=await run(s);assert.notEqual(r.status,0);assert.match(r.stderr,/IMAGE_|SEASON_|ASSET_/);});
test('批次每次回查当前库，拒绝已变更设计和源文件',async t=>{const s=await setup(t);await applyChange(s.registry,{version:'series-asset-change-v1',baseRevision:1,requestId:'revise',stage:'asset-design',action:'revise',assetId:'A0001',asset:{...s.asset,design:{state:'TEXT_ADOPTED',text:'修订后的本季设计'}},scopeUses:[{episode:1,sceneId:'1-1'}],uses:[],reason:'实际设计修订',duplicateReview:'同一稳定ID'});let r=await run(s);assert.notEqual(r.status,0);assert.match(r.stderr,/IMAGE_DESIGN/);const s2=await setup(t);await writeFile(s2.source,'改动源材料');r=await run(s2);assert.notEqual(r.status,0);assert.match(r.stderr,/IMAGE_SOURCE/);});
test('可移植季范围模块与独立Skill同一实现，没有第二套范围规则',async()=>{assert.equal(await readFile(join(base,'scripts/season-scope.mjs'),'utf8'),await readFile(join(base,'../series-storyboard-director/scripts/season-scope.mjs'),'utf8'));});

test('当前库无关新增不会让已采用设计和本批任务失效',async t=>{const s=await setup(t);await applyChange(s.registry,{version:'series-asset-change-v1',baseRevision:1,requestId:'unrelated',stage:'asset-design',action:'add',asset:{...s.asset,name:'无关路人',key:'无关路人CH',design:{state:'UNADOPTED',text:''}},scopeUses:[{episode:1,sceneId:'1-1'}],uses:[],reason:'本季另项登记',duplicateReview:'独立身份'});const r=await run(s);assert.equal(r.status,0,r.stderr);assert.equal(JSON.parse(r.stdout).tasks,1);});
