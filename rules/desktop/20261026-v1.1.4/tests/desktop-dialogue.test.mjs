import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdtemp,symlink} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {dirname,join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {inspectDialogue,materializeDialogue,verifyMaterialized} from '../scripts/materialize-dialogue.mjs';
import {checkBundle} from '../scripts/storyboard.mjs';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const source=Buffer.from(JSON.stringify({key:'E1',sourceRows:[
 {id:'E1:R1',text:'甲（OS）：先走😀，别回头。'},
 {id:'E1:R2',text:'乙：好。'}
],contextRows:[]}));
function selection(raw=source){
 const info=inspectDialogue(raw);
 return {version:'desktop-dialogue-selection-v1',coordinateProfile:info.coordinateProfile,sourceSha256:info.sourceSha256,
  draft:{schemaVersion:'performance-core-v4',units:[{id:'U1',shots:info.dialogue.map((q,i)=>({id:`S${i}`,dialogueParts:[
   {lineId:q.lineId,quoteStart:0,quoteEnd:q.utf16Length,presentation:q.channel==='OS'?'OS':'画内',delivery:'自然承接',voiceStart:0,voiceEnd:2}
  ]}))}]}};
}
const fails=(fn,code)=>assert.throws(fn,e=>e.code===code);

test('cross-shot exact words, punctuation and OS remain bound to source',()=>{
 const s=selection(),first=s.draft.units[0].shots[0],part=first.dialogueParts[0],q=inspectDialogue(source).dialogue[0];
 first.dialogueParts=[{...part,quoteEnd:4}];
 s.draft.units[0].shots.splice(1,0,{id:'S0b',dialogueParts:[{...part,quoteStart:4}]});
 const out=materializeDialogue(source,s);
 assert.equal(out.draft.units[0].shots.slice(0,2).flatMap(x=>x.dialogueParts).map(p=>p.text).join(''),q.quote);
 assert.equal(out.draft.units[0].shots[0].dialogueParts[0].presentation,'OS');
 assert.equal(out.draft.units[0].shots[0].dialogueParts[0].quoteStart,undefined);
 assert.equal(verifyMaterialized(source,out).ok,true);
});
test('source bytes/profile and model-supplied source fields cannot drift',()=>{
 const s=selection();fails(()=>materializeDialogue(Buffer.concat([source,Buffer.from('\n')]),s),'SOURCE_DRIFT');
 s.coordinateProfile='absolute-utf16';fails(()=>materializeDialogue(source,s),'COORDINATE_PROFILE');
 for(const field of ['text','speaker','channel','kind','performanceCues']){
  const v=selection();v.draft.units[0].shots[0].dialogueParts[0][field]='覆盖';
  fails(()=>materializeDialogue(source,v),'MODEL_SOURCE_FIELDS');
 }
});
test('invalid ranges, surrogate split, skipped and repeated words are rejected',()=>{
 for(const [start,end,code] of [[0,3,'UTF16_BOUNDARY'],[1,4,'DIALOGUE_GAP'],[0,999,'RANGE'],[0,0,'RANGE'],[0,1.5,'RANGE']]){
  const s=selection();Object.assign(s.draft.units[0].shots[0].dialogueParts[0],{quoteStart:start,quoteEnd:end});
  fails(()=>materializeDialogue(source,s),code);
 }
 const s=selection();s.draft.units[0].shots[0].dialogueParts.push({...s.draft.units[0].shots[0].dialogueParts[0]});
 fails(()=>materializeDialogue(source,s),'DIALOGUE_GAP');
});
test('scope is explicit and ordered; full episode catches missing dialogue',()=>{
 const s=selection(),info=inspectDialogue(source);s.draft.units[0].shots.pop();
 fails(()=>materializeDialogue(source,s),'DIALOGUE_COVERAGE');
 s.scopeLineIds=[info.dialogue[0].lineId];assert.equal(materializeDialogue(source,s).receipt.scopeLineIds.length,1);
 s.scopeLineIds=['unknown'];fails(()=>materializeDialogue(source,s),'SCOPE');
 s.scopeLineIds=info.dialogue.map(q=>q.lineId).reverse();fails(()=>materializeDialogue(source,s),'SCOPE');
});
test('a later line cannot bypass unfinished earlier speech',()=>{
 const s=selection();s.draft.units[0].shots.reverse();
 fails(()=>materializeDialogue(source,s),'DIALOGUE_GAP');
});
test('receipt, materialized text and draft tampering fail verification',()=>{
 const out=materializeDialogue(source,selection());
 const bad=structuredClone(out);bad.draft.units[0].shots[0].dialogueParts[0].text='改字';
 fails(()=>verifyMaterialized(source,bad),'DRAFT_DRIFT');
 const bad2=structuredClone(out);bad2.receipt.ranges[0].quoteEnd=1;
 assert.throws(()=>verifyMaterialized(source,bad2));
 const bad3=structuredClone(out);bad3.receipt.ranges.push({...bad3.receipt.ranges[0]});
 fails(()=>verifyMaterialized(source,bad3),'RECEIPT');
});
test('standard inline performance remains metadata and never enters spoken quote',async()=>{
 const dir=await mkdtemp(join(tmpdir(),'desktop-voice-')),textFile=join(dir,'source.txt'),known=join(dir,'speakers.json'),out=join(dir,'source.json');
 await writeFile(textFile,'第1集\n1-1 夜 内 门厅\n人物：甲\n甲：我（压低声音）听见了。\n');
 await writeFile(known,'["甲"]');
 const run=spawnSync(process.execPath,[join(root,'scripts/prepare-source.mjs'),'--text',textFile,'--key','E1','--known-speakers',known,'--output',out],{encoding:'utf8'});
 assert.equal(run.status,0,run.stdout+run.stderr);
 const raw=await readFile(out),info=inspectDialogue(raw),result=materializeDialogue(raw,selection(raw));
 assert.equal(result.draft.units[0].shots[0].dialogueParts[0].text,'我听见了。');
 assert.equal(info.dialogue[0].performanceCues[0].text,'压低声音');
 assert.equal(verifyMaterialized(raw,result).ok,true);
});
test('materialized draft fits the actual Lingmo bundle contract',async()=>{
 const bundle=JSON.parse(await readFile(join(root,'tests/fixtures/lingmo-v7-bundle.json'),'utf8'));
 for(const ep of bundle.episodes){
  const raw=Buffer.from(JSON.stringify(ep.source)),info=inspectDialogue(raw),draft=structuredClone(ep.draft),cursors=new Map();
  for(const u of draft.units)for(const s of u.shots)for(const p of s.dialogueParts){
   const start=cursors.get(p.lineId)??0;const end=start+p.text.length;cursors.set(p.lineId,end);
   delete p.text;p.quoteStart=start;p.quoteEnd=end;
  }
  ep.draft=materializeDialogue(raw,{version:'desktop-dialogue-selection-v1',coordinateProfile:info.coordinateProfile,sourceSha256:info.sourceSha256,draft}).draft;
 }
 assert.doesNotThrow(()=>checkBundle(bundle));
});
test('CLI is offline, works outside package cwd, and refuses existing output',async()=>{
 const dir=await mkdtemp(join(tmpdir(),'desktop-voice-cli-')),src=join(dir,'s.json'),sel=join(dir,'a.json'),out=join(dir,'o.json');
 await writeFile(src,source);await writeFile(sel,JSON.stringify(selection()));
 const args=[join(root,'scripts/materialize-dialogue.mjs'),'build','--source',src,'--input',sel,'--output',out];
 const r=spawnSync(process.execPath,args,{encoding:'utf8',cwd:tmpdir()});assert.equal(r.status,0,r.stderr);
 const before=await readFile(out);const again=spawnSync(process.execPath,args,{encoding:'utf8',cwd:tmpdir()});
 assert.notEqual(again.status,0);assert.deepEqual(await readFile(out),before);
 const v=spawnSync(process.execPath,[args[0],'verify','--source',src,'--input',out],{encoding:'utf8'});assert.equal(v.status,0,v.stderr);
 const alias=join(dir,'skill-link');
 await symlink(root,alias,process.platform==='win32'?'junction':'dir');
 const linked=spawnSync(process.execPath,[join(alias,'scripts/materialize-dialogue.mjs'),'verify','--source',src,'--input',out],{encoding:'utf8',cwd:tmpdir()});
 assert.equal(linked.status,0,linked.stderr);
 assert.equal(JSON.parse(linked.stdout).ok,true,'目录链接不能静默exit0而未执行工具');
});
