// Development-only oracle: frozen 0.1.2 tools, synthetic files, no production IO.
import {readFileSync,writeFileSync,mkdtempSync,mkdirSync,rmSync} from 'node:fs';
import {join,dirname,resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {pathToFileURL,fileURLToPath} from 'node:url';
const repo=resolve(dirname(fileURLToPath(import.meta.url)),'../../../..');
const scripts=join(repo,'rules/series-lite/20261027-v0.1.2/scripts');
const {hashSeasonScope}=await import(pathToFileURL(join(scripts,'season-scope.mjs')));
const {buildSeasonHandoff}=await import(pathToFileURL(join(scripts,'season-handoff.mjs')));
const {projectBundle,renderProjection}=await import(pathToFileURL(join(scripts,'storyboard.mjs')));
const hash=s=>createHash('sha256').update(s).digest('hex');
const encode=v=>JSON.stringify(v,null,2)+'\n';
const clone=structuredClone;
function put(files,path,value){files[path]=typeof value==='string'?value:encode(value);return {path};}
function edit(files,path,fn){const v=JSON.parse(files[path]);fn(v);files[path]=encode(v);}
function materialize(root,files){for(const [name,body] of Object.entries(files)){mkdirSync(dirname(join(root,name)),{recursive:true});writeFileSync(join(root,name),body);}}
const original='第1集\n合成源正文\n第2集\n下一季正文\n第3集\n最后正文\n',sourceHash=hash(original);
const reading=[{number:1,sceneIds:['1-1','1-2']},{number:2,sceneIds:['2-1']},{number:3,sceneIds:['3-1']}];
const scope={version:'series-season-scope-v1',id:'S1',revision:1,seasonNumber:1,status:'locked',sourceHash,readingScope:reading,episodes:[reading[0]],target:{minSeconds:7200,maxSeconds:9000,planningSeconds:8100},ending:{episode:1,sceneId:'1-2',sourceRef:'E01:R4',closure:'本次守门任务已完成。',hook:'门外新来者尚未现身。',estimate:{minSeconds:7200,maxSeconds:8600}},confirmation:{kind:'delegated',basis:'合成选择'},durationException:{basis:'合成十秒，不是实际季长',confirmation:{kind:'delegated',basis:'合成授权'}}};
const index={version:'series-source-v1',sourceHash,episodes:reading.map(e=>({number:e.number,scenes:e.sceneIds.map(id=>({id,sourceRefs:['E01:R1']}))}))};
function workspace(scope=null){
 const files={};put(files,'source-original.txt',original);put(files,'source-body.txt',original);
 put(files,'extraction.json',{version:'series-extraction-v1',sourceCopy:'source-original.txt',sha256:sourceHash,bodyHash:sourceHash});
 const ix=clone(index);if(scope){Object.assign(ix,{episodes:ix.episodes.filter(e=>scope.episodes.some(s=>s.number===e.number)),productionScope:scope,readingScope:reading,scopeHash:hashSeasonScope(scope)});}
 put(files,'source-index.json',ix);const numbers=ix.episodes.map(e=>e.number);
 const state={version:'series-workspace-v1',sourceHash,sourceIndexHash:hash(files['source-index.json']),scope:numbers,sourceOrder:reading.map(e=>e.number),groups:[{id:1,episodes:numbers}],episodes:Object.fromEntries(numbers.map(n=>[n,{status:'pending',group:1}])),history:[]};
 if(scope)Object.assign(state,{productionScope:scope,scopeHash:hashSeasonScope(scope)});
 put(files,'state.json',state);return files;
}
const old={};const oldRequest={version:'series-season-handoff-v1',kind:'closed',source:{original:put(old,'source.txt',original),body:put(old,'source-body.txt',original),index:put(old,'source-index.json',index)},scope:put(old,'scope.json',scope)};
const bundle=JSON.parse(readFileSync(join(scripts,'../tests/fixtures/lingmo-v7-bundle.json'),'utf8'));
Object.assign(bundle,{testFixture:false,sourceHash,productionScope:scope,scopeHash:hashSeasonScope(scope),seasonDelivery:'final'});
const assets=bundle.assets.map((a,i)=>({...a,id:`A${String(i+1).padStart(4,'0')}`,revision:1,aliases:[],scopeUses:scope.episodes[0].sceneIds.map(sceneId=>({episode:1,sceneId})),origin:{kind:'source',sourceRefs:['E01:R1'],basis:'合成文字'},design:{state:'TEXT_ADOPTED',text:'完整合成设计。'}}));
bundle.assets=bundle.assets.map((a,i)=>({...a,assetId:assets[i].id,assetRevision:1}));
const registry={version:'series-assets-v1',id:'R1',revision:1,importProfile:'lingmo-v7',sourceHash,productionScope:scope,productionScopeHash:hashSeasonScope(scope),assets,requests:[]};bundle.assetRegistry={id:'R1',revision:1};
const lock={version:'series-source-lock-v1',sourceHash,episodes:bundle.episodes.map(({number,source})=>({number,source}))};
bundle.sourceLockHash=hash(JSON.stringify(lock));bundle.generationTextVersion='voice-delivery-v1';
const unit=bundle.episodes.at(-1).draft.units.at(-1),shot=unit.shots.at(-1);
const continuity={version:'series-season-continuity-v1',scopeHash:hashSeasonScope(scope),tail:{episode:1,sceneId:unit.sceneId,unitId:unit.id,shotId:shot.id},states:[],stateBasis:'合成空走廊尾态',openHooks:[{text:'下一季原生钩子',sourceRefs:['E01:R4']}],deferred:[],unresolved:[],review:{kind:'delegated',basis:'合成闭季记录'}};
oldRequest.closed={bundle:put(old,'bundle.json',bundle),sourceLock:put(old,'lock.json',lock),registry:put(old,'registry.json',registry),adoptedStoryboard:put(old,'adopted.md',renderProjection(projectBundle(bundle,{registry,sourceLock:lock}))),continuity:put(old,'continuity.json',continuity),adoption:{kind:'delegated',basis:'合成采用，不是用户作品'},images:[]};
const temp=mkdtempSync(join(tmpdir(),'lite-workspace-oracle-'));
let handoff;try {materialize(temp,old);handoff=await buildSeasonHandoff(oldRequest,{root:temp});}finally{rmSync(temp,{recursive:true,force:true});}
put(old,'handoff.json',handoff);
const next={...clone(scope),id:'S2',seasonNumber:2,episodes:[reading[1]],ending:{...scope.ending,episode:2,sceneId:'2-1'},continuation:handoff.resume};
const bases={legacy:workspace(),first:workspace(scope),next:workspace(next)};
for(const [name,body] of Object.entries(old))bases.next['previous/'+name]=body;
edit(bases.next,'state.json',s=>s.continuationEvidence={path:'previous/handoff.json',sha256:hash(bases.next['previous/handoff.json']),previousScopeHash:handoff.scopeHash,nextEpisode:2});
const cases=[];
function add(name,base,fn=()=>{},unsupported=false){const files=clone(bases[base]);fn(files);const patches=Object.fromEntries(Object.entries(files).filter(([k,v])=>bases[base][k]!==v));const remove=Object.keys(bases[base]).filter(k=>!(k in files));cases.push({name,base,patches,remove,unsupported,files});}
function bindState(files){edit(files,'state.json',s=>s.sourceIndexHash=hash(files['source-index.json']));}
function bindHandoff(files){edit(files,'state.json',s=>s.continuationEvidence.sha256=hash(files['previous/handoff.json']));}
function refEdit(files,key,fn){const path='previous/'+handoff.closed[key].path;edit(files,path,fn);edit(files,'previous/handoff.json',h=>h.closed[key].sha256=hash(files[path]));bindHandoff(files);}
for(const base of Object.keys(bases))add(base+'-valid',base);
add('legacy-extra-state','legacy',f=>edit(f,'state.json',s=>s.anyUnrelated={keep:'verbatim'}));
add('legacy-source-copy-relative-dot','legacy',f=>edit(f,'extraction.json',e=>e.sourceCopy='./source-original.txt'));
for(const [name,file] of [['original','source-original.txt'],['body','source-body.txt'],['index','source-index.json']])add(name+'-drift','legacy',f=>f[file]+=' ');
for(const file of ['state.json','extraction.json','source-original.txt','source-body.txt','source-index.json'])add('missing-'+file,'legacy',f=>delete f[file]);
add('unknown-state-version','legacy',f=>edit(f,'state.json',s=>s.version='new'));
add('source-hash-mismatch','legacy',f=>edit(f,'state.json',s=>s.sourceHash='a'.repeat(64)));
add('state-json-malformed','legacy',f=>f['state.json']='{');
add('source-escape','legacy',f=>edit(f,'extraction.json',s=>s.sourceCopy='../outside.txt'));
for(const key of ['scopeHash','readingScope'])add('partial-index-'+key,'legacy',f=>{edit(f,'source-index.json',s=>s[key]=[]);bindState(f);});
add('partial-state-season','legacy',f=>edit(f,'state.json',s=>s.productionScope=null));
add('unexpected-legacy-prior','legacy',f=>edit(f,'state.json',s=>s.continuationEvidence={path:'previous/handoff.json'}));
add('unexpected-first-prior','first',f=>edit(f,'state.json',s=>s.continuationEvidence={path:'previous/handoff.json'}));
add('first-scope-empty','first',f=>edit(f,'state.json',s=>s.scope=[]));
add('first-scope-outside','first',f=>edit(f,'state.json',s=>s.scope=[2]));
add('first-scope-duplicate','first',f=>edit(f,'state.json',s=>s.scope=[1,1]));
add('first-group-missing','first',f=>edit(f,'state.json',s=>s.groups=[]));
add('first-episode-status-missing','first',f=>edit(f,'state.json',s=>s.episodes={}));
add('first-state-scope-drift','first',f=>edit(f,'state.json',s=>s.productionScope.revision++));
add('first-index-scopehash-drift','first',f=>{edit(f,'source-index.json',s=>s.scopeHash='a'.repeat(64));bindState(f);});
add('first-index-scenes-drift','first',f=>{edit(f,'source-index.json',s=>s.episodes[0].scenes.pop());bindState(f);});
add('first-unlocked','first',f=>{edit(f,'source-index.json',s=>s.productionScope.status='proposed');bindState(f);});
add('first-numeric-json-season-valid','first',f=>{f['source-index.json']=f['source-index.json'].replace('"seasonNumber": 1','"seasonNumber": 1.0');bindState(f);f['state.json']=f['state.json'].replace('"seasonNumber": 1','"seasonNumber": 1.0');});
add('next-missing-prior','next',f=>edit(f,'state.json',s=>delete s.continuationEvidence));
add('next-prior-drift','next',f=>f['previous/handoff.json']+=' ');
add('next-prior-extra-evidence','next',f=>edit(f,'state.json',s=>s.continuationEvidence.extra=true));
add('next-prior-path-escape','next',f=>edit(f,'state.json',s=>s.continuationEvidence.path='../outside.json'));
add('next-planned','next',f=>{edit(f,'previous/handoff.json',h=>h.kind='planned');bindHandoff(f);});
add('next-handoff-unknown-field','next',f=>{edit(f,'previous/handoff.json',h=>h.extra=true);bindHandoff(f);});
for(const key of ['scopeHash','resume','actualTail','reusableAssets'])add('next-derived-'+key,'next',f=>{edit(f,'previous/handoff.json',h=>h[key]=null);bindHandoff(f);});
for(const key of ['original','body','index'])add('next-source-'+key+'-drift','next',f=>f['previous/'+handoff.source[key].path]+=' ');
for(const key of ['bundle','sourceLock','registry','adoptedStoryboard','continuity'])add('next-'+key+'-drift','next',f=>f['previous/'+handoff.closed[key].path]+=' ');
add('next-bundle-fixture','next',f=>refEdit(f,'bundle',b=>b.testFixture=true));
add('next-bundle-duration','next',f=>refEdit(f,'bundle',b=>b.episodes[0].draft.units[0].shots[0].duration=1));
add('next-bundle-lost-unit','next',f=>refEdit(f,'bundle',b=>b.episodes[0].draft.units.pop()));
add('next-registry-version','next',f=>refEdit(f,'registry',r=>r.assets[0].revision++));
add('next-lock-source-rewrite','next',f=>refEdit(f,'sourceLock',r=>r.episodes[0].source.sourceRows[0].text='改写'));
for(const [name,fn] of [['unresolved',v=>v.unresolved=['尚未解决']],['tail',v=>v.tail.shotId='wrong'],['refs',v=>v.openHooks[0].sourceRefs=['invented']],['review',v=>v.review=null],['no-basis',v=>delete v.stateBasis],['state-unknown',v=>v.states=[{assetId:'unknown'}]]])add('next-continuity-'+name,'next',f=>refEdit(f,'continuity',fn));
add('next-continuity-bom-review','next',f=>refEdit(f,'continuity',v=>v.review.basis='\ufeff'));
add('next-md-rehashed-tamper','next',f=>{f['previous/adopted.md']=f['previous/adopted.md'].replace('门厅','不存在');edit(f,'previous/handoff.json',h=>h.closed.adoptedStoryboard.sha256=hash(f['previous/adopted.md']));bindHandoff(f);});
add('next-image-candidate-valid','next',f=>{f['previous/image.txt']='synthetic';edit(f,'previous/handoff.json',h=>h.closed.images=[{assetId:assets[0].id,assetRevision:1,state:'candidate_pending_user',file:{path:'image.txt',sha256:hash('synthetic')}}]);bindHandoff(f);});
add('next-image-adopted-valid','next',f=>{f['previous/image.txt']='synthetic';edit(f,'previous/handoff.json',h=>h.closed.images=[{assetId:assets[0].id,assetRevision:1,state:'adopted',file:{path:'image.txt',sha256:hash('synthetic')},adoption:{kind:'delegated',basis:'合成审图记录'}}]);bindHandoff(f);});
add('next-image-adoption-missing','next',f=>{edit(f,'previous/handoff.json',h=>h.closed.images=[{assetId:assets[0].id,assetRevision:1,state:'adopted'}]);bindHandoff(f);});
add('next-handoff-path-escape','next',f=>{edit(f,'previous/handoff.json',h=>h.closed.bundle.path='../source-index.json');bindHandoff(f);});
// The frozen tool permits old single-zone closed packages; native adapter declines explicitly.
add('next-legacy-single-zone-unsupported','next',f=>{
 const b=clone(bundle);delete b.exportProfile;const r=clone(oldRequest);r.closed.bundle={path:'bundle.json'};r.closed.adoptedStoryboard={path:'adopted.md'};
 const oldFiles=clone(old);put(oldFiles,'bundle.json',b);put(oldFiles,'adopted.md',renderProjection(projectBundle(b,{registry,sourceLock:lock})));
 f['previous/bundle.json']=oldFiles['bundle.json'];f['previous/adopted.md']=oldFiles['adopted.md'];
 edit(f,'previous/handoff.json',h=>{h.closed.bundle.sha256=hash(f['previous/bundle.json']);h.closed.adoptedStoryboard.sha256=hash(f['previous/adopted.md']);});bindHandoff(f);
},true);
for(const c of cases){
 const root=mkdtempSync(join(tmpdir(),'lite-ws-case-'));try{materialize(root,c.files);const result=spawnSync('python',['-B','-c','import sys,json;sys.path.insert(0,sys.argv[1]);from pathlib import Path;from workspace import load_state;print(json.dumps(load_state(Path(sys.argv[2]).resolve()),ensure_ascii=False))',scripts,root],{encoding:'utf8',timeout:30000});if(result.error)throw result.error;if(![0,1].includes(result.status))throw new Error('oracle process did not complete: '+result.status);c.oracleOk=result.status===0;c.oracleError=c.oracleOk?null:(result.stderr||result.error?.message||'failed').split('\n').filter(Boolean).at(-1);if(c.oracleOk)c.stateRaw=c.files['state.json'];}finally{rmSync(root,{recursive:true,force:true});}delete c.files;
}
const out={sourceCommit:'6dea8d6c57dde38f04a4d71f05f4c039004dd4a9',bases,cases};
writeFileSync(join(repo,'apps/desktop/src-tauri/src/series_lite_workspace_golden.json'),encode(out));
console.log(JSON.stringify({cases:cases.length,passedOracle:cases.filter(c=>c.oracleOk).length,refusedOracle:cases.filter(c=>!c.oracleOk).length,unsupported:cases.filter(c=>c.unsupported).map(c=>[c.name,c.oracleOk])}));
