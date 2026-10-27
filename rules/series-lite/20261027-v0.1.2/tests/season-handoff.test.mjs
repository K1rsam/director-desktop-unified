import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdtemp,rm,mkdir,symlink} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {buildSeasonHandoff,verifySeasonHandoff,verifyContinuationScope} from '../scripts/season-handoff.mjs';

const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const save=async(root,path,value)=>{await writeFile(join(root,path),typeof value==='string'?value:JSON.stringify(value,null,2)+'\n');return {path};};
async function setup(){
 const root=await mkdtemp(join(tmpdir(),'season-handoff-'));
 const original='第1集\n合成源正文\n第2集\n下一季正文\n',sourceHash=hash(original);
 const scope={version:'series-season-scope-v1',id:'S1',revision:1,seasonNumber:1,status:'locked',sourceHash,
  readingScope:[{number:1,sceneIds:['1-1','1-2']},{number:2,sceneIds:['2-1']}],episodes:[{number:1,sceneIds:['1-1','1-2']}],
  target:{minSeconds:7200,maxSeconds:9000,planningSeconds:8100},ending:{episode:1,sceneId:'1-2',sourceRef:'E01:R4',closure:'本次守门任务已完成。',hook:'门外新来者尚未现身。',estimate:{minSeconds:7200,maxSeconds:8600}},confirmation:{kind:'delegated',basis:'合成用例中明确委托选择此尾。'},durationException:{basis:'合成样例只有10秒；仅验证交接机制。',confirmation:{kind:'delegated',basis:'测试授权例外，不是实际季长。'}}};
 const index={version:'series-source-v1',sourceHash,episodes:scope.readingScope.map(e=>({number:e.number,scenes:e.sceneIds.map(id=>({id,sourceRefs:['E01:R1']}))}))};
 const input={version:'series-season-handoff-v1',kind:'planned',source:{original:await save(root,'source.txt',original),body:await save(root,'source-body.txt',original),index:await save(root,'source-index.json',index)},scope:await save(root,'season-scope.json',scope)};
 return {root,input,scope,index};
}
async function closing(ctx){
 const {hashSeasonScope}=await import('../scripts/season-scope.mjs');
 const {projectBundle,renderProjection}=await import('../scripts/storyboard.mjs');
 const {root,input,scope}=ctx;
 const bundle=JSON.parse(await readFile(new URL('./fixtures/export-bundle.json',import.meta.url),'utf8'));
 bundle.testFixture=false;bundle.sourceHash=scope.sourceHash;bundle.productionScope=scope;bundle.scopeHash=hashSeasonScope(scope);bundle.seasonDelivery='final';
 const assets=bundle.assets.map((a,i)=>({...a,id:`A${String(i+1).padStart(4,'0')}`,revision:1,aliases:[],scopeUses:scope.episodes[0].sceneIds.map(sceneId=>({episode:1,sceneId})),origin:{kind:'source',sourceRefs:['E01:R1'],basis:'合成文字来源'},design:{state:'TEXT_ADOPTED',text:'完整沿用合成样例的设计。'}}));
 bundle.assets=bundle.assets.map((a,i)=>({...a,assetId:assets[i].id,assetRevision:1}));
 const registry={version:'series-assets-v1',id:'R1',revision:1,sourceHash:scope.sourceHash,productionScope:scope,productionScopeHash:hashSeasonScope(scope),assets,requests:[]};
 bundle.assetRegistry={id:registry.id,revision:1};
 const sourceLock={version:'series-source-lock-v1',sourceHash:scope.sourceHash,episodes:bundle.episodes.map(({number,source})=>({number,source}))};
 bundle.sourceLockHash=hash(JSON.stringify(sourceLock));bundle.generationTextVersion='voice-delivery-v1';
 const unit=bundle.episodes.at(-1).draft.units.at(-1),shot=unit.shots.at(-1);
 const continuity={version:'series-season-continuity-v1',scopeHash:hashSeasonScope(scope),tail:{episode:1,sceneId:'1-2',unitId:unit.id,shotId:shot.id},states:[],stateBasis:'末场空走廊；守门者的最后实际状态见前场采用镜。',openHooks:[{text:'门外新来者尚未现身。',sourceRefs:['E01:R4']}],deferred:['第2集才出现的资产和镜头不制作。'],unresolved:[],review:{kind:'delegated',basis:'测试只证明字段/范围/哈希一致，不证明真实内容采用。'}};
 input.kind='closed';input.closed={bundle:await save(root,'bundle.json',bundle),sourceLock:await save(root,'source-lock.json',sourceLock),registry:await save(root,'registry.json',registry),adoptedStoryboard:await save(root,'采用分镜.md',renderProjection(projectBundle(bundle,{registry,sourceLock}))),continuity:await save(root,'continuity.json',continuity),adoption:{kind:'delegated',basis:'合成用例授权采用文字分镜，非用户的实际稿。'},images:[]};
 return {bundle,registry,sourceLock,continuity};
}
const sandbox=fn=>async()=>{const ctx=await setup();try{await fn(ctx);}finally{await rm(ctx.root,{recursive:true,force:true});}};

test('计划包冻结源与季范围并给出下一未制作原集，但不是闭季证据',sandbox(async ctx=>{
 const handoff=await buildSeasonHandoff(ctx.input,{root:ctx.root});
 assert.equal(handoff.kind,'planned');assert.equal(handoff.scope.sha256.length,64);assert.deepEqual(handoff.resume,{previousSeasonId:'S1',previousScopeHash:handoff.scopeHash,nextEpisode:2});
 const report=await verifySeasonHandoff(handoff,{root:ctx.root});assert.equal(report.readyForContinuation,false);assert.equal(report.actualTail,null);
 handoff.kind='closed';await assert.rejects(verifySeasonHandoff(handoff,{root:ctx.root}),{code:'HANDOFF_CLOSED'});
}));
test('闭季需真实逐镜覆盖、锁源、资产快照和采用稿一致，报告实际秒数与例外',sandbox(async ctx=>{
 const {bundle}=await closing(ctx);const handoff=await buildSeasonHandoff(ctx.input,{root:ctx.root});
 const report=await verifySeasonHandoff(handoff,{root:ctx.root});
 assert.equal(report.readyForContinuation,true);assert.equal(report.coverage.complete,true);assert.equal(report.duration.actualSeconds,10);assert.equal(report.duration.status,'exception-authorized');
 const unit=bundle.episodes[0].draft.units.at(-1);assert.equal(report.actualTail.unitId,unit.id);assert.equal(report.actualTail.exitState,unit.exitState);assert.equal(report.reusableAssets[0].assetId,'A0001');
 assert.match(report.proof,/不证明/);
}));
test('目录完整或手填总秒数不能关闭缺镜的季',sandbox(async ctx=>{
 const {bundle}=await closing(ctx);bundle.episodes[0].draft.units.pop();bundle.totalSeconds=8100;
 await save(ctx.root,'bundle.json',bundle);
 await assert.rejects(buildSeasonHandoff(ctx.input,{root:ctx.root}),/完整|覆盖|场|范围/);
}));
test('未授权时长越界、合成标记和未解决承接冲突均不能签实际闭季',sandbox(async ctx=>{
 const {bundle,continuity}=await closing(ctx);
 const noException=structuredClone(ctx.scope);delete noException.durationException;
 await save(ctx.root,'season-scope.json',noException);bundle.productionScope=noException;
 const {hashSeasonScope}=await import('../scripts/season-scope.mjs');bundle.scopeHash=hashSeasonScope(noException);await save(ctx.root,'bundle.json',bundle);
 await assert.rejects(buildSeasonHandoff(ctx.input,{root:ctx.root}),{code:'SEASON_DURATION_TARGET'});
 await save(ctx.root,'season-scope.json',ctx.scope);bundle.productionScope=ctx.scope;bundle.scopeHash=hashSeasonScope(ctx.scope);bundle.testFixture=true;await save(ctx.root,'bundle.json',bundle);
 await assert.rejects(buildSeasonHandoff(ctx.input,{root:ctx.root}),{code:'HANDOFF_CLOSED'});
 bundle.testFixture=false;await save(ctx.root,'bundle.json',bundle);continuity.unresolved=['人物何时受伤尚未核清'];await save(ctx.root,'continuity.json',continuity);
 await assert.rejects(buildSeasonHandoff(ctx.input,{root:ctx.root}),{code:'HANDOFF_CONTINUITY'});
}));
test('即使重新计算文件hash，改变采用MD也不能冒充同一bundle的收口稿',sandbox(async ctx=>{
 await closing(ctx);await writeFile(join(ctx.root,'采用分镜.md'),'仅有结尾摘要，缺少全量采用分镜。\n');
 await assert.rejects(buildSeasonHandoff(ctx.input,{root:ctx.root}),/语法|反解|第|标准化|一致/);
}));
test('实际采用稿与尾态坐标不一致、缺源锚或候选图标为已采用均拒绝',sandbox(async ctx=>{
 const {continuity}=await closing(ctx);continuity.tail.shotId='wrong';await save(ctx.root,'continuity.json',continuity);
 await assert.rejects(buildSeasonHandoff(ctx.input,{root:ctx.root}),{code:'HANDOFF_TAIL'});
 continuity.tail.shotId=JSON.parse(await readFile(join(ctx.root,'bundle.json'),'utf8')).episodes[0].draft.units.at(-1).shots.at(-1).id;
 continuity.openHooks[0].sourceRefs=['invented'];await save(ctx.root,'continuity.json',continuity);
 await assert.rejects(buildSeasonHandoff(ctx.input,{root:ctx.root}),{code:'HANDOFF_CONTINUITY'});
 continuity.openHooks[0].sourceRefs=['E01:R4'];await save(ctx.root,'continuity.json',continuity);
 ctx.input.closed.images=[{assetId:'A0001',assetRevision:1,state:'adopted',file:await save(ctx.root,'candidate.txt','candidate image placeholder')}];
 await assert.rejects(buildSeasonHandoff(ctx.input,{root:ctx.root}),{code:'HANDOFF_IMAGE_ADOPTION'});
}));
test('源/资产/采用文本变化必须重新收口，不沿用旧hash',sandbox(async ctx=>{
 await closing(ctx);const handoff=await buildSeasonHandoff(ctx.input,{root:ctx.root});
 await writeFile(join(ctx.root,'采用分镜.md'),'被修改');
 await assert.rejects(verifySeasonHandoff(handoff,{root:ctx.root}),{code:'HANDOFF_FILE_HASH'});
}));
test('续季只能从已闭季的下一原集接续，拒绝计划包、重做、错hash及新源假迁移',sandbox(async ctx=>{
 const planned=await buildSeasonHandoff(ctx.input,{root:ctx.root});
 await assert.rejects(verifyContinuationScope(planned,{}, {root:ctx.root}),{code:'HANDOFF_NOT_CLOSED'});
 await closing(ctx);const handoff=await buildSeasonHandoff(ctx.input,{root:ctx.root});
 const next=structuredClone(ctx.scope);next.id='S2';next.seasonNumber=2;next.episodes=[next.readingScope[1]];next.ending.episode=2;next.ending.sceneId='2-1';next.continuation=handoff.resume;
 assert.equal((await verifyContinuationScope(handoff,next,{root:ctx.root})).nextEpisode,2);
 next.continuation={...handoff.resume,previousScopeHash:'f'.repeat(64)};
 await assert.rejects(verifyContinuationScope(handoff,next,{root:ctx.root}),{code:'HANDOFF_CONTINUATION'});
 next.continuation=handoff.resume;next.episodes=[next.readingScope[0]];
 await assert.rejects(verifyContinuationScope(handoff,next,{root:ctx.root}),/续|连续|原集|开始/);
 next.sourceHash='f'.repeat(64);
 await assert.rejects(verifyContinuationScope(handoff,next,{root:ctx.root}),{code:'HANDOFF_SOURCE_TRANSITION'});
}));
test('交接包拒绝只剩本季的投影索引，即使它携带完整readingScope与合法季hash',sandbox(async ctx=>{
 await closing(ctx);const {hashSeasonScope}=await import('../scripts/season-scope.mjs');
 const projected={...ctx.index,episodes:ctx.index.episodes.slice(0,1),readingScope:ctx.scope.readingScope,productionScope:ctx.scope,scopeHash:hashSeasonScope(ctx.scope)};
 await save(ctx.root,'source-index.json',projected);
 await assert.rejects(buildSeasonHandoff(ctx.input,{root:ctx.root}),{code:'HANDOFF_FULL_INDEX'});
}));
test('完整原始索引可只嵌入上季范围；核续季使用只读全剧视图，不继承旧季绑定',sandbox(async ctx=>{
 await closing(ctx);const embedded={...ctx.index,productionScope:ctx.scope};await save(ctx.root,'source-index.json',embedded);
 const handoff=await buildSeasonHandoff(ctx.input,{root:ctx.root}),before=await readFile(join(ctx.root,'source-index.json'),'utf8');
 const next=structuredClone(ctx.scope);next.id='S2';next.seasonNumber=2;next.episodes=[next.readingScope[1]];next.ending.episode=2;next.ending.sceneId='2-1';next.continuation=handoff.resume;
 assert.equal((await verifyContinuationScope(handoff,next,{root:ctx.root})).nextEpisode,2);
 assert.equal(await readFile(join(ctx.root,'source-index.json'),'utf8'),before);
}));
test('包中引用必须在对接目录内，拒绝绝对路径、上跳和符号链接外逃',sandbox(async ctx=>{
 for(const path of ['/tmp/outside','../outside']){ctx.input.scope={path};await assert.rejects(buildSeasonHandoff(ctx.input,{root:ctx.root}),{code:'HANDOFF_PATH'});}
 await symlink(tmpdir(),join(ctx.root,'escape'));ctx.input.scope={path:'escape/outside.json'};
 await assert.rejects(buildSeasonHandoff(ctx.input,{root:ctx.root}),/路径|目录|不存在/);
}));
test('CLI create 拒绝覆盖，verify 只读回查；EOF 不伪造下一集',sandbox(async ctx=>{
 ctx.scope.readingScope=ctx.scope.episodes;ctx.index.episodes=ctx.index.episodes.slice(0,1);await save(ctx.root,'season-scope.json',ctx.scope);await save(ctx.root,'source-index.json',ctx.index);
 await save(ctx.root,'request.json',ctx.input);const script=fileURLToPath(new URL('../scripts/season-handoff.mjs',import.meta.url));
 const args=['create','--input',join(ctx.root,'request.json'),'--root',ctx.root,'--output',join(ctx.root,'handoff.json')];
 let result=spawnSync(process.execPath,[script,...args],{encoding:'utf8'});assert.equal(result.status,0,result.stderr);
 const handoff=JSON.parse(await readFile(join(ctx.root,'handoff.json'),'utf8'));assert.equal(handoff.resume.nextEpisode,null);
 result=spawnSync(process.execPath,[script,...args],{encoding:'utf8'});assert.notEqual(result.status,0);
 result=spawnSync(process.execPath,[script,'verify','--input',join(ctx.root,'handoff.json'),'--root',ctx.root],{encoding:'utf8'});assert.equal(result.status,0,result.stderr);
}));
