// Separate visual-review contract. Never upgrades a frozen text-only attempt.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { pathToFileURL, fileURLToPath } from 'node:url';
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const fail = code => { throw new Error(code); };
const requireThat = (value,code) => { if(!value)fail(code); };
// Same pinned ErrorItem contract as the text bridge: a startup/in-turn
// diagnostic grants no tool, turn, model result, completion or replay.
function diagnosticItem(event) {
  const item=event.item;
  if(event.type!=='item.completed'||item===null||typeof item!=='object'||Array.isArray(item)||item.type!=='error'
    ||typeof item.id!=='string'||!/^[A-Za-z0-9_-]{1,128}$/.test(item.id)||typeof item.message!=='string'
    ||!item.message.length||Buffer.byteLength(item.message)>4096)return false;
  return Object.keys(event).sort().join(',')==='item,type'&&Object.keys(item).sort().join(',')==='id,message,type';
}
export const canonical = v => Array.isArray(v)?v.map(canonical):v&&typeof v==='object'?Object.fromEntries(Object.keys(v).sort().map(k=>[k,canonical(v[k])])):v;
const bytes = v => JSON.stringify(canonical(v));
function atomic(file,value) {const data=bytes(value),temp=file+'.tmp';const fd=fs.openSync(temp,'wx',0o600);try{fs.writeFileSync(fd,data);fs.fsyncSync(fd);}finally{fs.closeSync(fd);}fs.renameSync(temp,file);}
function safeFile(file,root) {
  requireThat(path.isAbsolute(file)&&path.dirname(fs.realpathSync(file))===path.join(fs.realpathSync(root),'images'),'VISUAL_FILE_SCOPE');
  requireThat(!fs.lstatSync(file).isSymbolicLink()&&fs.statSync(file).isFile(),'VISUAL_FILE_LINK');
  const relative=path.relative(root,file);requireThat(!relative.startsWith('..')&&!path.isAbsolute(relative),'VISUAL_FILE_SCOPE');
  let current=root;for(const part of relative.split(path.sep)){current=path.join(current,part);requireThat(!fs.lstatSync(current).isSymbolicLink(),'VISUAL_FILE_LINK');}
}
export function verifyImages(manifest,root) {
  requireThat(Array.isArray(manifest.images)&&manifest.images.length>=1&&manifest.images.length<=12,'VISUAL_INPUT_COUNT');
  for(const [index,image]of manifest.images.entries()) {
    requireThat(image.index===index+1&&['candidate','reference','probe'].includes(image.role),'VISUAL_INPUT_ROLE');
    safeFile(image.path,root);const stat=fs.statSync(image.path);
    requireThat(stat.size===image.bytes&&stat.size>0&&stat.size<=20*1024*1024&&/^[a-f0-9]{64}$/.test(image.sha256),'VISUAL_INPUT_BYTES');
    requireThat(hash(fs.readFileSync(image.path))===image.sha256,'VISUAL_INPUT_CHANGED');
  }
}
export function visualInputs(manifest) {
  // Pinned SDK concatenates all text and then adds --image paths in order.
  // The prompt therefore identifies inputs by explicit 1-based ordinal.
  return [{type:'text',text:manifest.prompt},...manifest.images.map(i=>({type:'local_image',path:i.path}))];
}
export async function runFrozen(file,expectedHash,createSdk=null) {
  const root=path.dirname(file),source=fs.readFileSync(file),manifest=JSON.parse(source);
  requireThat(hash(source)===expectedHash&&manifest.version==='visual-review-attempt/v1'&&manifest.permissionProfile==='sdk-visual-review-native/v1','VISUAL_MANIFEST_IDENTITY');
  requireThat(['capability','candidate'].includes(manifest.purpose)&&manifest.model==='gpt-6.1-sol'&&manifest.reasoning==='medium','VISUAL_MODEL_OR_PURPOSE');
  requireThat(manifest.runnerSha256===hash(fs.readFileSync(fileURLToPath(import.meta.url))),'VISUAL_RUNNER_IDENTITY');
  const receiptFile=path.join(root,'sdk-receipt.json'),eventFile=path.join(root,'events.ndjson');
  requireThat(!fs.existsSync(receiptFile)&&!fs.existsSync(eventFile),'VISUAL_ATTEMPT_EXISTS');
  verifyImages(manifest,root);
  const sdkPath=path.join(manifest.baseIdentity.resourceDirectory,'node_modules/@openai/codex-sdk/dist/index.js');
  requireThat(hash(fs.readFileSync(sdkPath))===manifest.baseIdentity.sdkImplementationSha256,'VISUAL_SDK_IDENTITY');
  requireThat(hash(fs.readFileSync(manifest.baseIdentity.supervisorExecutable))===manifest.baseIdentity.supervisorSha256,'VISUAL_SUPERVISOR_IDENTITY');
  requireThat(hash(fs.readFileSync(manifest.baseIdentity.runtimeExecutable))===manifest.baseIdentity.runtimeSha256,'VISUAL_RUNTIME_IDENTITY');
  const receipt={version:'visual-sdk-receipt/v1',attemptId:manifest.attemptId,manifestSha256:expectedHash,state:'dispatching',threadId:null,
    finalText:null,usage:null,imageCount:manifest.images.length,inputHashes:manifest.images.map(i=>i.sha256),error:null,model:manifest.model,reasoning:manifest.reasoning,visualInputAttached:true,domainAccepted:false};
  atomic(receiptFile,receipt);
  const journal=fs.openSync(eventFile,'wx',0o600),controller=new AbortController();let size=0,count=0,started=false,turn=false,done=false,final=null;
  const timer=setTimeout(()=>controller.abort(),180000);
  try {
    const env={DIRECTOR_VISUAL_PLAN:file,DIRECTOR_VISUAL_SHA256:expectedHash,USERPROFILE:manifest.baseIdentity.credentialHome,
      HOME:manifest.baseIdentity.credentialHome,TEMP:path.join(root,'tmp'),TMP:path.join(root,'tmp'),TMPDIR:path.join(root,'tmp')};
    if(process.platform==='win32'){env.SystemRoot=process.env.SystemRoot;env.WINDIR=process.env.SystemRoot;}
    const options={codexPathOverride:manifest.baseIdentity.supervisorExecutable,env};
    const sdk=createSdk?createSdk(options):new (await import(pathToFileURL(sdkPath))).Codex(options);
    const thread=sdk.startThread({model:manifest.model,modelReasoningEffort:manifest.reasoning,workingDirectory:path.join(root,'work'),sandboxMode:'read-only',approvalPolicy:'never',networkAccessEnabled:false,webSearchMode:'disabled',skipGitRepoCheck:true});
    const stream=await thread.runStreamed(visualInputs(manifest),{outputSchema:manifest.schema,signal:controller.signal});
    for await(const event of stream.events) {
      const line=JSON.stringify(event)+'\n';size+=Buffer.byteLength(line);count++;
      requireThat(size<=8*1024*1024&&count<=1024&&Buffer.byteLength(line)<=1024*1024,'VISUAL_EVENT_LIMIT');
      fs.writeSync(journal,line);fs.fsyncSync(journal);requireThat(!done,'VISUAL_EVENT_AFTER_COMPLETION');
      if(event.type==='thread.started'){requireThat(!started&&typeof event.thread_id==='string','VISUAL_THREAD_ORDER');started=true;receipt.threadId=event.thread_id;}
      else if(event.type==='turn.started'){requireThat(started&&!turn,'VISUAL_TURN_ORDER');turn=true;}
      else if(started&&diagnosticItem(event)){ /* Durable diagnostic, no model output. */ }
      else if(['item.started','item.updated','item.completed'].includes(event.type)) {
        requireThat(started&&turn&&['agent_message','reasoning'].includes(event.item?.type),'VISUAL_TOOL_BLOCKED');
        if(event.type==='item.completed'&&event.item.type==='agent_message'){requireThat(typeof event.item.text==='string','VISUAL_FINAL_TEXT');final=event.item.text;}
      } else if(event.type==='turn.completed'){requireThat(turn&&typeof final==='string'&&event.usage&&typeof event.usage==='object','VISUAL_INCOMPLETE');done=true;receipt.usage=event.usage;}
      else if(event.type==='error'&&typeof event.message==='string'&&/^Reconnecting\.\.\. [1-9][0-9]?\/[1-9][0-9]? \(stream disconnected before completion: [^\r\n]+\)$/.test(event.message)){requireThat(turn,'VISUAL_RECONNECT_ORDER');}
      else fail(event.type==='turn.failed'?'VISUAL_TURN_FAILED':'VISUAL_UNSUPPORTED_EVENT');
    }
    requireThat(done,'VISUAL_INCOMPLETE');verifyImages(manifest,root);
    requireThat(hash(fs.readFileSync(file))===expectedHash,'VISUAL_MANIFEST_CHANGED');
    receipt.finalText=final;receipt.finalTextSha256=hash(final);receipt.state='transport_completed';
  } catch(e) {controller.abort();receipt.state='submission_unknown';receipt.error=String(e.message).slice(0,512);}
  finally {clearTimeout(timer);fs.closeSync(journal);receipt.eventsSha256=hash(fs.readFileSync(eventFile));receipt.eventCount=count;atomic(receiptFile,receipt);}
  return receipt;
}
if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url)) {
  try {const result=await runFrozen(process.argv[2],process.argv[3]);process.stdout.write(JSON.stringify({state:result.state,attemptId:result.attemptId}));}
  catch(e){process.stderr.write(String(e.message).slice(0,512));process.exitCode=1;}
}
