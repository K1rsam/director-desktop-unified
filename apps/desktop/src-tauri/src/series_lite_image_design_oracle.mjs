// Development-only frozen Node oracle. Never used by the installed client.
import {readFile, writeFile, mkdtemp, rm} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {tmpdir} from 'node:os';
import {join, resolve, dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {validateDesignLevel, validateImageDesigns, resolveImageDesign, prepareImageDesignBatch} from '../../../../rules/series-lite/20261028-v0.4.1/scripts/asset-image-design.mjs';
import {recordImageDesign, readRegistry} from '../../../../rules/series-lite/20261028-v0.4.1/scripts/assets.mjs';

const hash = value => createHash('sha256').update(value).digest('hex');
const clone = structuredClone;
const design = (body = '同一身份；袖口可抓握', level = 'storyboard') => ({state:'TEXT_ADOPTED',text:body,full:{body},...(level === undefined ? {} : {detailLevel:level})});
const asset = (id, name, deps = [], level = 'storyboard') => ({id,revision:1,name,key:name+'CH',type:'CH',status:'常态',aliases:[],origin:{kind:'source',sourceRefs:['E1:R1'],basis:'合成源'},design:{...design(undefined,level),dependencies:deps}});
const base = (legacy = false) => ({version:'series-assets-v1',id:'synthetic-image-registry',revision:1,sourceHash:'a'.repeat(64),assets:[asset('A0001','主角',[],legacy?undefined:'storyboard'),asset('A0002','主角婚礼',[{assetId:'A0001',assetRevision:1,purpose:'同一身份'}],legacy?undefined:'storyboard'),asset('A0003','主角婚礼带Q版',[{assetId:'A0002',assetRevision:1,purpose:'正常婚礼与对应3D Q同图'}],legacy?undefined:'storyboard'),asset('A0004','旁人',[],legacy?undefined:'storyboard')],requests:[],changes:[]});
// JS default parameters replace undefined; remove the marker explicitly here.
const legacyBase = () => { const r=base();for(const a of r.assets)delete a.design.detailLevel;return r; };
const request = (revision = 1, ids = ['A0001'], id = 'image-1') => ({version:'series-image-design-v1',baseRevision:revision,requestId:id,reason:'补充制图布局，分镜约束保持',entries:ids.map(assetId=>({assetId,assetRevision:1,design:design('完整制图外观和布局','image'),compatibilityBasis:'身份、动作和空间条件保全'}))});
const append = (registry,input) => {const r=clone(registry),b=prepareImageDesignBatch(r,input);r.revision++;r.imageDesigns=[...(r.imageDesigns??[]),b];return r;};
const full = append(base(),request(1,['A0003','A0002','A0001']));
const cases=[];
async function run(op, payload) {
 if(op==='level'){validateDesignLevel(payload.asset);return true;}
 if(op==='validate'){validateImageDesigns(payload.registry);return true;}
 if(op==='resolve')return resolveImageDesign(payload.registry,payload.assetId);
 if(op==='prepare')return prepareImageDesignBatch(payload.registry,payload.input);
 if(op==='hash')return hash(JSON.stringify(payload.value,(_,x)=>x&&typeof x==='object'&&!Array.isArray(x)?Object.fromEntries(Object.keys(x).sort().map(k=>[k,x[k]])):x));
 const directory=await mkdtemp(join(tmpdir(),'lite041-image-oracle-'));
 try{
  const bytes=JSON.stringify(payload.registry,null,2)+'\n';
  await writeFile(join(directory,'v'+String(payload.registry.revision).padStart(6,'0')+'.json'),bytes);
  await writeFile(join(directory,'current.json'),JSON.stringify({version:'series-assets-head-v1',id:payload.registry.id,revision:payload.registry.revision,sha256:hash(bytes)}));
  const result=await recordImageDesign(directory,payload.input),after=(await readRegistry(directory)).registry;
  return {registry:after,changed:result.changed,assetRevisionsChanged:result.assetRevisionsChanged,requestId:result.requestId,...(result.entries===undefined?{}:{entries:result.entries})};
 }finally{
  const resolved=resolve(directory),prefix=resolve(tmpdir())+'\\';
  if(process.platform==='win32'&&!resolved.startsWith(prefix))throw Error('temporary cleanup escaped temp root');
  await rm(resolved,{recursive:true,force:true});
 }
}
async function add(name,op,payload){
 const item={name,op,...clone(payload)};
 try{item.expected={ok:true,value:await run(op,clone(payload))};}catch(e){item.expected={ok:false,code:e.code??'UNCLASSIFIED'};}
 cases.push(item);
}
await add('legacy-unmarked-design','level',{asset:legacyBase().assets[0]});
for(const [name,mutate] of [
 ['adopted-storyboard-full',()=>{}],['adopted-image-full',a=>a.design.detailLevel='image'],['missing-full',a=>delete a.design.full],['empty-full',a=>a.design.full.body=''],['null-level',a=>a.design.detailLevel=null],['unknown-level',a=>a.design.detailLevel='short-card'],['unadopted-no-full',a=>{a.design.state='UNADOPTED';delete a.design.full;}],['unicode-blank-body',a=>a.design.full.body='\uFEFF\u3000'],['literal-cr-body',a=>a.design.full.body='x\r']]){
 const a=clone(base().assets[0]);mutate(a);await add(name,'level',{asset:a});
}
await add('no-image-records','validate',{registry:base()});
await add('null-image-records','validate',{registry:{...base(),imageDesigns:null}});
await add('storyboard-not-image-ready','resolve',{registry:base(),assetId:'A0001'});
await add('legacy-three-layer-resolution','resolve',{registry:legacyBase(),assetId:'A0003'});
await add('unknown-resolution','resolve',{registry:base(),assetId:'A9999'});
await add('reverse-parent-batch','prepare',{registry:base(),input:request(1,['A0003','A0002','A0001'])});
await add('reverse-parent-record','record',{registry:base(),input:request(1,['A0003','A0002','A0001'])});
await add('current-history-valid','validate',{registry:full});
await add('current-child-resolution','resolve',{registry:full,assetId:'A0003'});
await add('request-idempotent-old-base','record',{registry:full,input:request(1,['A0003','A0002','A0001'])});
for(const [name,mutate] of [
 ['forbidden-root-field',r=>r.assets=[]],['forbidden-entry-field',r=>r.entries[0].status='changed'],['stale-base',r=>r.baseRevision=9],['old-asset-revision',r=>r.entries[0].assetRevision=9],['unknown-id',r=>r.entries[0].assetId='A9999'],['duplicate-id',r=>r.entries.push(clone(r.entries[0]))],['missing-full-body',r=>delete r.entries[0].design.full.body],['unadopted-art',r=>r.entries[0].design.state='UNADOPTED'],['storyboard-art',r=>r.entries[0].design.detailLevel='storyboard'],['changed-narrative-dependency',r=>r.entries[0].design.dependencies=[{assetId:'A0003',assetRevision:1,purpose:'偷改'}]],['null-dependencies',r=>r.entries[0].design.dependencies=null],['missing-batch-reason',r=>delete r.reason],['noninteger-base',r=>r.baseRevision=1.1]]){
 const input=request();mutate(input);await add(name,'record',{registry:base(),input});
}
await add('no-parent-art','record',{registry:base(),input:request(1,['A0003'])});
const collision=base();collision.changes.push({id:'image-1'});await add('request-other-change-collision','record',{registry:collision,input:request()});
const conflict=request(1,['A0003','A0002','A0001']);conflict.entries[0].design.text+='changed';await add('idempotent-content-conflict','record',{registry:full,input:conflict});
const parentReq=request(2,['A0002'],'wedding-layout');parentReq.entries[0].design.full.body+='；新布局';
const parentChanged=append(full,parentReq);
await add('updated-parent-valid-history','validate',{registry:parentChanged});
await add('updated-parent-child-stale','resolve',{registry:parentChanged,assetId:'A0003'});
await add('updated-parent-root-unaffected','resolve',{registry:parentChanged,assetId:'A0001'});
await add('updated-parent-art-is-current','resolve',{registry:parentChanged,assetId:'A0002'});
await add('old-request-not-current','record',{registry:parentChanged,input:request(1,['A0003','A0002','A0001'])});
await add('reaccept-only-child','record',{registry:parentChanged,input:request(3,['A0003'],'q-confirmed')});
const legacy=legacyBase(),rootOnly=append(legacy,request());
await add('legacy-child-new-parent-stale','resolve',{registry:rootOnly,assetId:'A0003'});
await add('legacy-child-reaccept-direct-chain','record',{registry:rootOnly,input:request(2,['A0003','A0002'],'derived')});
for(const [name,mutate] of [
 ['record-version-gap',r=>r.imageDesigns[0].entries[1].imageDesignRevision=2],['record-body-missing',r=>delete r.imageDesigns[0].entries[0].design.full.body],['record-future-asset',r=>r.imageDesigns[0].entries[0].assetRevision=2],['record-duplicate-asset',r=>r.imageDesigns[0].entries.push(clone(r.imageDesigns[0].entries[0]))],['record-forbidden-field',r=>r.imageDesigns[0].entries[0].status='x'],['record-bad-parent-hash',r=>r.imageDesigns[0].entries[1].dependencyImages[0].designHash='b'.repeat(64)],['record-self-parent',r=>r.imageDesigns[0].entries[1].dependencyImages[0].assetId='A0002'],['record-duplicate-batch-id',r=>r.imageDesigns.push(clone(r.imageDesigns[0]))],['record-other-id-conflict',r=>r.usageAnnotations=[{id:'image-1'}]],['record-past-registry-revision',r=>r.revision=1]]){
 const registry=clone(full);mutate(registry);await add(name,'validate',{registry});
}
const narrativeChanged=clone(full);narrativeChanged.assets[1].revision=2;await add('narrative-change-art-stale','resolve',{registry:narrativeChanged,assetId:'A0002'});
const cycle=legacyBase();cycle.assets[0].design.dependencies=[{assetId:'A0003',assetRevision:1,purpose:'cycle'}];
await add('resolve-cycle','resolve',{registry:cycle,assetId:'A0003'});await add('prepare-cycle','prepare',{registry:cycle,input:request(1,['A0003','A0002','A0001'])});
const unknownParent=base();unknownParent.assets[0].design.dependencies=[{assetId:'A9999',assetRevision:1,purpose:'missing'}];await add('unknown-parent','prepare',{registry:unknownParent,input:request()});
const staleParent=legacyBase();staleParent.assets[0].revision=2;await add('stale-parent-narrative','prepare',{registry:staleParent,input:request(1,['A0002'])});
await add('numeric-unicode-stable-hash','hash',{value:{'😀':-0,'a':1.0,'\uE000':1e-7,body:'完整设计',unknown:{n:123456789.12345679}}});
const eleven=base();eleven.assets=[];
for(let n=1;n<=11;n++)eleven.assets.push(asset('A'+String(n).padStart(4,'0'),'小龟形态'+n,n===1?[]:[{assetId:'A'+String(n-1).padStart(4,'0'),assetRevision:1,purpose:'继承上一层真实形态'}]));
const elevenRequest=request(1,eleven.assets.map(a=>a.id).reverse(),'eleven-layers');
await add('eleven-layer-reverse-record','record',{registry:eleven,input:elevenRequest});
await add('eleven-layer-resolution','resolve',{registry:append(eleven,elevenRequest),assetId:'A0011'});
const here=dirname(fileURLToPath(import.meta.url));
const source=await readFile(new URL('../../../../rules/series-lite/20261028-v0.4.1/scripts/asset-image-design.mjs',import.meta.url));
const golden={sourceCommit:'f855227c8406b38093d03c777fe24a0b0b4f87dc',sourceSha256:hash(source),cases};
await writeFile(join(here,'series_lite_image_design_golden.json'),JSON.stringify(golden,null,2)+'\n');
console.log(JSON.stringify({cases:cases.length,success:cases.filter(c=>c.expected.ok).length,rejected:cases.filter(c=>!c.expected.ok).length,unclassified:cases.filter(c=>c.expected.code==='UNCLASSIFIED').map(c=>c.name),sourceSha256:golden.sourceSha256}));
