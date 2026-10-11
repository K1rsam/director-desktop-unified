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
