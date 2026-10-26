import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {runFrozen,verifyImages,visualInputs} from '../src/visual-review.mjs';
const sha=v=>createHash('sha256').update(v).digest('hex');
const here=path.dirname(fileURLToPath(import.meta.url));
function fixture(t){
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'director-visual-test-'));
  t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  for(const sub of ['images','work','tmp','sdk/node_modules/@openai/codex-sdk/dist'])fs.mkdirSync(path.join(root,sub),{recursive:true});
  const sdk=path.join(root,'sdk/node_modules/@openai/codex-sdk/dist/index.js');fs.writeFileSync(sdk,'offline injected fixture');
  const executable=path.join(root,'fixture-executable');fs.writeFileSync(executable,'never executed');
  const images=[1,2].map(index=>{const bytes=Buffer.from(`synthetic image ${index}`),file=path.join(root,'images',`${index}.png`);fs.writeFileSync(file,bytes);return{index,role:'probe',path:file,sha256:sha(bytes),bytes:bytes.length};});
  const manifest={version:'visual-review-attempt/v1',permissionProfile:'sdk-visual-review-native/v1',attemptId:'offline-fixture',purpose:'capability',model:'gpt-6.1-sol',reasoning:'medium',images,prompt:'Describe each attachment in input order. No tools.',schema:{type:'object'},runnerSha256:sha(fs.readFileSync(path.join(here,'../src/visual-review.mjs'))),baseIdentity:{resourceDirectory:path.join(root,'sdk'),sdkImplementationSha256:sha(fs.readFileSync(sdk)),supervisorExecutable:executable,supervisorSha256:sha(fs.readFileSync(executable)),runtimeExecutable:executable,runtimeSha256:sha(fs.readFileSync(executable)),credentialHome:root}};
  const file=path.join(root,'manifest.json');const save=()=>{fs.writeFileSync(file,JSON.stringify(manifest));return sha(fs.readFileSync(file));};
  return{root,manifest,file,save};
}
function sdk(events,capture=()=>{}){return options=>({startThread(threadOptions){capture({options,threadOptions});return{async runStreamed(inputs,turnOptions){capture({inputs,turnOptions});return{events:(async function*(){for(const event of events)yield event;})()};}};}});}
const completed=[{type:'thread.started',thread_id:'offline-thread'},{type:'turn.started'},{type:'item.completed',item:{type:'agent_message',text:'{"observed":true}'}},{type:'turn.completed',usage:{input_tokens:5,output_tokens:3,cached_input_tokens:0}}];
test('visual-only request attaches ordered local images and records transport, never adoption',async t=>{
  const f=fixture(t),seen=[];const receipt=await runFrozen(f.file,f.save(),sdk(completed,v=>seen.push(v)));
  assert.equal(receipt.state,'transport_completed');assert.equal(receipt.domainAccepted,false);assert.equal(receipt.visualInputAttached,true);
  assert.deepEqual(seen[1].inputs,[{type:'text',text:f.manifest.prompt},...f.manifest.images.map(v=>({type:'local_image',path:v.path}))]);
  assert.equal(seen[0].threadOptions.sandboxMode,'read-only');assert.equal(seen[0].threadOptions.webSearchMode,'disabled');
  assert.equal(receipt.eventsSha256,sha(fs.readFileSync(path.join(f.root,'events.ndjson'))));
  assert.deepEqual(receipt.inputHashes,f.manifest.images.map(v=>v.sha256));
  await assert.rejects(runFrozen(f.file,f.save(),sdk(completed)),/VISUAL_ATTEMPT_EXISTS/);
});
for(const mutation of ['bytes','outside','role','order','count'])test(`reject changed or out-of-contract image: ${mutation}`,t=>{
  const f=fixture(t);
  if(mutation==='bytes')fs.appendFileSync(f.manifest.images[0].path,'changed');
  if(mutation==='outside'){const file=path.join(f.root,'work','unapproved.png');fs.copyFileSync(f.manifest.images[0].path,file);f.manifest.images[0].path=file;}
  if(mutation==='role')f.manifest.images[0].role='unregistered';
  if(mutation==='order')f.manifest.images.reverse();
  if(mutation==='count')f.manifest.images=[];
  assert.throws(()=>verifyImages(f.manifest,f.root),/VISUAL_/);
});
test('directory junction cannot expose external input',t=>{
  const f=fixture(t),outside=fs.mkdtempSync(path.join(os.tmpdir(),'director-visual-outside-'));t.after(()=>fs.rmSync(outside,{recursive:true,force:true}));
  const image=f.manifest.images[0];fs.copyFileSync(image.path,path.join(outside,'foreign.png'));
  const link=path.join(f.root,'images','linked');try{fs.symlinkSync(outside,link,process.platform==='win32'?'junction':'dir');}catch(e){t.skip(`symlink unavailable: ${e.code}`);return;}
  image.path=path.join(link,'foreign.png');assert.throws(()=>verifyImages(f.manifest,f.root),/VISUAL_FILE_/);
});
for(const [name,events]of [
  ['tools',[...completed.slice(0,2),{type:'item.started',item:{type:'command_execution',command:'read'}}]],
  ['incomplete',completed.slice(0,3)],['no-thread',completed.slice(1)],['after-completion',[...completed,{type:'turn.started'}]],
  ['turn-failed',[...completed.slice(0,2),{type:'turn.failed',error:{message:'network'}}]],
])test(`ambiguous ${name} stays UNKNOWN and original cannot be resent`,async t=>{
  const f=fixture(t),receipt=await runFrozen(f.file,f.save(),sdk(events));
  assert.equal(receipt.state,'submission_unknown');assert.equal(receipt.domainAccepted,false);
  await assert.rejects(runFrozen(f.file,f.save(),sdk(completed)),/VISUAL_ATTEMPT_EXISTS/);
});
test('literal input path is not model visible text and no interleaved image assumptions',t=>{
  const f=fixture(t);assert.equal(visualInputs(f.manifest).filter(v=>v.type==='text').length,1);
  assert.equal(visualInputs(f.manifest)[0].text.includes(f.root),false);
});
const warning={type:'item.completed',item:{id:'item_0',type:'error',message:'Under-development features enabled: skip_host_skill_discovery. Under-development features are incomplete and may behave unpredictably.'}};
for(const position of [1,2])test(`strict diagnostic at stream position ${position} preserves the same completed stream`,async t=>{
  const f=fixture(t),events=[...completed.slice(0,position),warning,...completed.slice(position)];let calls=0;
  const receipt=await runFrozen(f.file,f.save(),sdk(events,v=>{if(v.inputs)calls++;}));
  assert.equal(receipt.state,'transport_completed');assert.equal(calls,1);assert.equal(receipt.finalText,completed[2].item.text);
  assert.deepEqual(fs.readFileSync(path.join(f.root,'events.ndjson'),'utf8').trim().split('\n').map(JSON.parse),events);
  assert.equal(receipt.domainAccepted,false);
  await assert.rejects(runFrozen(f.file,f.save(),sdk(completed)),/VISUAL_ATTEMPT_EXISTS/);
});
for(const [name,event]of [
  ['missing-id',{type:'item.completed',item:{type:'error',message:'warning'}}],
  ['extra-tool-field',{...warning,item:{...warning.item,command:'run something'}}],
  ['extra-event-field',{...warning,tool:'execute'}],
  ['started-diagnostic',{...warning,type:'item.started'}],
  ['empty-message',{...warning,item:{...warning.item,message:''}}],
  ['oversized-utf8',{...warning,item:{...warning.item,message:'错'.repeat(1500)}}],
  ['tool-disguised',{...warning,item:{...warning.item,type:'command_execution'}}],
])test(`malformed startup diagnostic ${name} stays UNKNOWN`,async t=>{
  const f=fixture(t),receipt=await runFrozen(f.file,f.save(),sdk([completed[0],event,...completed.slice(1)]));
  assert.equal(receipt.state,'submission_unknown');assert.equal(receipt.finalText,null);assert.equal(receipt.domainAccepted,false);
});
for(const [name,events]of [
  ['before-thread',[warning,...completed]],
  ['after-completion',[...completed,warning]],
  ['diagnostic-only',[completed[0],warning]],
])test(`diagnostic ${name} is not a completed model turn`,async t=>{
  const f=fixture(t),receipt=await runFrozen(f.file,f.save(),sdk(events));
  assert.equal(receipt.state,'submission_unknown');assert.equal(receipt.domainAccepted,false);
});
