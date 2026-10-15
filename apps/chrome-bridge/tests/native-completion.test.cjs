'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),{createHash}=require('node:crypto');
const source=fs.readFileSync(path.join(__dirname,'../overrides/src/browserBridge.js'),'utf8');
const body=source.slice(source.indexOf('export function nativeImageCompletionPayload'),source.indexOf('export class BrowserBridge')).replace('export function','function');
const context=vm.createContext({createHash,URL});vm.runInContext(body+';this.complete=nativeImageCompletionPayload',context);
const complete=context.complete;
const id='12345678-1234-1234-1234-123456789abc',sha=createHash('sha256').update('prompt with spaces').digest('hex');
function fixture(){
  const identity={requestId:id,leaseId:'lease',ownerServerInstanceId:'server',responseEpoch:0};
  const state={requestId:id,clientId:'owned-source',promptPayload:{requestId:id,message:'prompt  with\nspaces'},runtime:{finished:false,cancellationRequested:false}};
  const client={id:'owned-source',ready:true,compatible:true,url:'https://chatgpt.com/c/conversation',launchToken:'bridge-director-desktop-'+id,activeRequest:{...identity}};
  const options={sourceClientId:client.id,reconcileConversation:{mode:'image_record_v1',requestId:id,conversationId:'conversation',promptSha256:sha,imageCount:1}};
  const binding={artifactId:'artifact',imageFileId:'file-original',bytesSha256:'a'.repeat(64),recordSha256:'b'.repeat(64),recordRecheckSha256:'c'.repeat(64),proofSource:'native_record_download',mime:'image/png',size:100};
  const response={sourceClientId:client.id,requestId:'',url:client.url,session:{id:'conversation'},turnKey:'assistant',userTurnKey:'user',artifacts:[{id:'artifact',kind:'image',generatedImage:true,downloadable:true,size:100,mime:'image/png',sourceClientId:client.id,sourceTurnKey:'assistant',requestId:'recovered',downloadUrl:'https://chatgpt.com/backend-api/estuary/content?id=file-original'}],reconciliation:{source:'conversation-record',version:1,proofVersion:2,status:'matched_complete',requestId:id,promptSha256:sha,conversationId:'conversation',backendStatus:'finished_successfully',endTurn:true,userMessageId:'user',assistantMessageId:'assistant',currentNode:'final-node',deliveryNode:'image-node',deliveryMessageId:'image-message',branchIdentity:[{nodeKey:'final-node',id:'assistant',role:'assistant',status:'finished_successfully',endTurn:true},{nodeKey:'image-node',id:'image-message',role:'tool',status:'finished_successfully'},{nodeKey:'user-node',id:'user',role:'user'}],bindings:[binding]}};
  return {state,identity,client,options,response};
}
const run=f=>complete(f.state,f.identity,f.client,f.options,f.response);
test('exact native proof binds the original pending request despite distinct node/message IDs and recovery envelope',()=>{
  for(const url of ['https://chatgpt.com/backend-api/estuary/content?id=file-original','https://chatgpt.com/backend-api/estuary/content?file_id=file-original','https://chatgpt.com/backend-api/estuary/content/file-original']){
    const f=fixture();f.response.artifacts[0].downloadUrl=url;const out=run(f);
    assert.equal(out.requestId,id);assert.equal(out.artifacts[0].requestId,id);assert.equal(out.finishReason,'native_image_record_complete');
    assert.equal(out.completionEvidence.generationStopped,true);assert.equal(f.response.requestId,'');assert.equal(f.state.runtime.finished,false,'guard cannot mutate lifecycle');
  }
});
test('native completion rejects terminal history, stale ownership, retry epochs, edited prompts and mismatched file proofs',()=>{
  const mutations=[f=>{f.state.runtime.finished=true},f=>{f.state.runtime.cancellationRequested=true},f=>{f.state.promptPayload.message='different'},f=>{f.client.launchToken='other'},f=>{f.client.activeRequest.leaseId='other'},f=>{f.identity.responseEpoch=1;f.client.activeRequest.responseEpoch=1},f=>{f.client.activeRequest.ownerServerInstanceId='other'},f=>{f.client.ready=false},f=>{f.options.sourceClientId='other'},f=>{f.options.reconcileConversation.mode='image_record_probe'},f=>{f.response.sourceClientId='other'},f=>{f.response.requestId='other'},f=>{f.response.reconciliation.status='diagnostic_only'},f=>{f.response.reconciliation.promptSha256='d'.repeat(64)},f=>{f.response.reconciliation.currentNode='old-node'},f=>{f.response.reconciliation.branchIdentity[0].id='other'},f=>{f.response.reconciliation.branchIdentity[0].endTurn=false},f=>{f.response.reconciliation.branchIdentity.push({id:'another-user',role:'user'})},f=>{f.response.reconciliation.deliveryNode='absent'},f=>{f.response.turnKey='other'},f=>{f.response.userTurnKey='other'},f=>{f.response.artifacts[0].id='other'},f=>{f.response.artifacts[0].downloadUrl='https://example.com/backend-api/estuary/content?id=file-original'},f=>{f.response.artifacts[0].size=99},f=>{f.response.reconciliation.bindings[0].bytesSha256=''},f=>{f.response.reconciliation.bindings[0].recordRecheckSha256=''},f=>{f.response.reconciliation.bindings[0].proofSource='dom_native_url'},f=>{f.response.artifacts.push({...f.response.artifacts[0]})}];
  mutations.forEach((mutate,i)=>{const f=fixture();mutate(f);assert.equal(run(f),null,'mutation '+i)});
});
test('malformed native branch or binding returns a hold without throwing',()=>{
  for(const mutate of [f=>{f.response.reconciliation.branchIdentity=[]},f=>{f.response.reconciliation.branchIdentity.push(null)},f=>{f.response.reconciliation.bindings=[null]},f=>{f.response.artifacts=[null]}]){
    const f=fixture();mutate(f);assert.equal(run(f),null);
  }
});

function withPresentationTail(f){
  const p=f.response.reconciliation;
  p.branchIdentity.forEach((node,index,branch)=>{node.parent=index+1===branch.length?null:branch[index+1].nodeKey;});
  p.branchIdentity.unshift({nodeKey:'recap-node',parent:'final-node',id:'recap-message',role:'assistant',status:'finished_successfully',endTurn:false});
  p.currentNode='recap-node';
  p.presentationTail={version:1,kind:'reasoning_recap',nodeKey:'recap-node',messageId:'recap-message',parentNodeKey:'final-node',
    finalNodeKey:'final-node',finalMessageId:'assistant',contentSha256:'d'.repeat(64),contentValueType:'string',hasImagePointers:false,
    recipient:'all',channel:null,hideInlineActions:true,finalRecipient:'all',finalChannel:'final'};
  return f;
}
test('canonical completion accepts only an explicit presentation tail while retaining its real current branch',()=>{
  const f=withPresentationTail(fixture()),before=JSON.stringify(f.response.reconciliation);
  const result=run(f);assert.equal(result.finishReason,'native_image_record_complete');
  assert.equal(result.completionEvidence.nativeProof.currentNode,'recap-node');
  assert.equal(result.completionEvidence.nativeProof.branchIdentity[0].endTurn,false);
  assert.equal(result.completionEvidence.nativeProof.assistantMessageId,'assistant');
  assert.equal(JSON.stringify(f.response.reconciliation),before);
});
test('canonical presentation tail rejects unknown shape activity ownership links or incomplete evidence',()=>{
  const mutations=[
    p=>{delete p.presentationTail},p=>{p.presentationTail=null},p=>{p.presentationTail.version=2},p=>{p.presentationTail.kind='unknown'},
    p=>{p.presentationTail.contentValueType='object'},p=>{p.presentationTail.contentSha256=''},
    p=>{p.presentationTail.hasImagePointers=true},p=>{p.presentationTail.hideInlineActions=false},
    p=>{p.presentationTail.recipient='tool'},p=>{p.presentationTail.channel='analysis'},
    p=>{p.presentationTail.finalRecipient='tool'},p=>{p.presentationTail.finalChannel='analysis'},
    p=>{p.presentationTail.nodeKey='different'},p=>{p.presentationTail.messageId='different'},
    p=>{p.presentationTail.messageId='';p.branchIdentity[0].id=''},
    p=>{p.presentationTail.parentNodeKey='different'},p=>{p.presentationTail.finalNodeKey='different'},p=>{p.presentationTail.finalMessageId='different'},
    p=>{p.branchIdentity[0].endTurn=true},p=>{p.branchIdentity[0].status='in_progress'},p=>{p.branchIdentity[0].role='tool'},
    p=>{p.branchIdentity[0].parent='image-node'},p=>{p.branchIdentity[1].id='different'},
    p=>{p.branchIdentity[2].endTurn=true;p.branchIdentity[2].role='assistant'},
    p=>{p.currentNode='final-node'},p=>{p.branchIdentity.push({nodeKey:'duplicate',parent:null,id:'another-user',role:'user'})},
    p=>{p.branchIdentity[2].parent='not-current-branch'},p=>{p.branchIdentity.at(-1).parent='cycle'},
  ];
  mutations.forEach((mutate,index)=>{const f=withPresentationTail(fixture());mutate(f.response.reconciliation);assert.equal(run(f),null,`tail mutation ${index}`);});
  for(const role of ['assistant','tool','user']){
    const f=withPresentationTail(fixture()),p=f.response.reconciliation;
    p.branchIdentity.unshift({nodeKey:'real-later',parent:p.currentNode,id:'real-later-id',role,status:'finished_successfully',endTurn:role==='assistant'});
    p.currentNode='real-later';assert.equal(run(f),null,`later ${role}`);
  }
});
