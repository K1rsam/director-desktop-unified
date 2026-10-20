import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import os from 'node:os';import path from 'node:path';import vm from 'node:vm';import {createHash,webcrypto} from 'node:crypto';
import {attachReferenceFiles,preSendJournal,settlePreSendFailure,requestHash} from './bridge.mjs';
const sha=b=>createHash('sha256').update(b).digest('hex');
function fixture(t){const root=fs.mkdtempSync(path.join(os.tmpdir(),'director-ref-upload-'));t.after(()=>{assert.equal(path.dirname(root),fs.realpathSync(os.tmpdir()));fs.rmSync(root,{recursive:true,force:true});});return root;}
function browser(){
 let changes=0;const input={accept:'image/*',files:[],__reactProps$test:{onChange(){changes++;}}};
 class Transfer{constructor(){this.files=[];this.items={add:file=>this.files.push(file)};}}
 const context=vm.createContext({crypto:webcrypto,Uint8Array,Blob,File,atob,Event,DataTransfer:Transfer,document:{querySelectorAll:()=>[input]}});
 const sizes=[];return{input,changes:()=>changes,sizes,page:{async evaluate(code){sizes.push(Buffer.byteLength(code));return await vm.runInContext(code,context);}}};
}
test('large parent images upload in bounded commands with exact original bytes and one callback',async t=>{
 const dir=fixture(t),bytes=Buffer.alloc(2_253_852);for(let i=0;i<bytes.length;i++)bytes[i]=(i*17)%251;const file=path.join(dir,'parent.png');fs.writeFileSync(file,bytes);const ref={path:file,sha256:sha(bytes)},b=browser();
 assert.deepEqual(await attachReferenceFiles(b.page,[ref],'large_parent'),{count:1});assert.ok(b.sizes.length>15);assert.ok(Math.max(...b.sizes)<256*1024);assert.equal(b.changes(),1);assert.equal(b.input.files[0].name,'parent.png');assert.equal(sha(Buffer.from(await b.input.files[0].arrayBuffer())),ref.sha256);
 await attachReferenceFiles(b.page,[ref],'large_parent');assert.equal(b.changes(),1,'acknowledged upload must not invoke onChange again');
});
test('multiple reference images retain file order and never use native file input commands',async t=>{
 const dir=fixture(t),refs=['b.png','a.png'].map((name,i)=>{const p=path.join(dir,name),bytes=Buffer.alloc(140_001+i,i+31);fs.writeFileSync(p,bytes);return{path:p,sha256:sha(bytes)}}),b=browser();
 await attachReferenceFiles(b.page,refs,'ordered');assert.deepEqual(b.input.files.map(f=>f.name),['b.png','a.png']);assert.equal(b.changes(),1);
});
test('a lost upload acknowledgement cannot duplicate the attachment callback',async t=>{
 const dir=fixture(t),file=path.join(dir,'source.png'),bytes=Buffer.from('synthetic byte fixture');fs.writeFileSync(file,bytes);const b=browser(),evaluate=b.page.evaluate;let lost=false;
 b.page.evaluate=async code=>{const value=await evaluate(code);if(code.startsWith('(async()=>')&&!lost){lost=true;throw Error('synthetic acknowledgement lost');}return value;};
 const ref={path:file,sha256:sha(bytes)};await assert.rejects(attachReferenceFiles(b.page,[ref],'lost_ack'));await attachReferenceFiles(b.page,[ref],'lost_ack');assert.equal(b.changes(),1);
});
test('changed source bytes fail before any browser operation',async t=>{
 const dir=fixture(t),file=path.join(dir,'changed.png');fs.writeFileSync(file,'changed');const b=browser();await assert.rejects(attachReferenceFiles(b.page,[{path:file,sha256:'a'.repeat(64)}],'changed'));assert.equal(b.sizes.length,0);
});
function original(dir){const request={requestId:'test-pre-send',kind:'image',model:'chatgpt-image',prompt:'synthetic prompt',references:[]};const saved={version:'opencli-task/v1',requestId:request.requestId,requestHash:requestHash(request),request,status:'submission_unknown',page:'OWNED_PAGE_1',profile:'profile1',accountKey:'account1',conversationUrl:null,error:'synthetic upload interrupted'};const loc={file:path.join(dir,'request.json')};fs.writeFileSync(loc.file,JSON.stringify(saved));return{request,saved,loc};}
const empty={page:'OWNED_PAGE_1',url:'https://chatgpt.com/',accountKey:'account1',userCount:0,assistantCount:0,composerEmpty:true,generating:false};
test('two exact empty observations settle only the original pre-send task without sending',async t=>{
 const dir=fixture(t),{request,saved,loc}=original(dir),before=fs.readFileSync(loc.file);let reads=0;
 const result=await settlePreSendFailure({runtimeSha256:'b'.repeat(64)},request,saved,loc,null,async()=>{reads++;return{...empty}});
 assert.equal(reads,2);assert.equal(result.status,'failed');assert.equal(result.raw.taskSettlement.reason,'not_submitted');assert.equal(result.raw.opencliNotSubmitted.journalBeforeSha256,sha(before));assert.equal(result.raw.opencliNotSubmitted.journalAfterSha256,sha(fs.readFileSync(loc.file)));assert.notEqual(result.raw.opencliNotSubmitted.journalBeforeSha256,result.raw.opencliNotSubmitted.journalAfterSha256);assert.deepEqual(JSON.parse(fs.readFileSync(loc.file)).request,request);assert.equal(Object.hasOwn(JSON.parse(fs.readFileSync(loc.file)),'sentAt'),false);
});
test('any send marker, conversation or nonempty owned page preserves the original unknown',async t=>{
 const dir=fixture(t);
 for(const patch of [{sentAt:''},{sentAt:null},{sendConfirmed:false},{conversationUrl:'https://chatgpt.com/c/known'},{status:'processing'}]){const{request,saved,loc}=original(dir);Object.assign(saved,patch);fs.writeFileSync(loc.file,JSON.stringify(saved));const before=fs.readFileSync(loc.file);assert.equal(preSendJournal(saved),false);assert.equal(await settlePreSendFailure({},request,saved,loc,null,()=>assert.fail('no observation allowed')),null);assert.deepEqual(fs.readFileSync(loc.file),before);}
 for(const patch of [{userCount:1},{assistantCount:1},{accountKey:'other'},{composerEmpty:false},{generating:true},{page:'OTHER_PAGE'},{url:'https://chatgpt.com/c/unknown'}]){const{request,saved,loc}=original(dir),before=fs.readFileSync(loc.file);assert.equal(await settlePreSendFailure({},request,saved,loc,null,async()=>({...empty,...patch})),null);assert.deepEqual(fs.readFileSync(loc.file),before);}
});
test('a concurrent original journal change cannot be overwritten by settlement',async t=>{
 const dir=fixture(t),{request,saved,loc}=original(dir);let count=0;const changed=JSON.stringify({...saved,thirdParty:true});const result=await settlePreSendFailure({},request,saved,loc,null,async()=>{if(++count===2)fs.writeFileSync(loc.file,changed);return{...empty}});assert.equal(result,null);assert.equal(fs.readFileSync(loc.file,'utf8'),changed);
});
