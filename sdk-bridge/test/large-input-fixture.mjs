// Rust integration fixture only. Uses the real bridge, pipe, journal and pinned
// SDK with a synthetic child. No native runtime or account capability exists.
import { OfficialSdkBridge } from '../src/bridge.mjs';
import { offlineFixture } from './fixture-runtime.mjs';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { createInterface } from 'node:readline';
const args=process.argv.slice(2);
if (args.length!==4 || args[0]!=='--state-dir' || args[2]!=='--offline-fixture') throw Error('offline fixture arguments required');
const limit=4_194_304;
const descriptor={version:'sdk-large-input-controller/v1',controllerSha256:createHash('sha256').update(await readFile(new URL('../src/large-input-controller.mjs',import.meta.url))).digest('hex'),inputFrameBytes:limit,outputFrameBytes:1_048_576};
const send=value=>{const line=JSON.stringify(value)+'\n';if(Buffer.byteLength(line)>1_048_576)throw Error('output limit');process.stdout.write(line);};
const bridge=new OfficialSdkBridge({stateDirectory:args[1],offlineFixture:offlineFixture(args[3]==='probe-fails'?'happy':args[3]),emit:send,limits:{frameBytes:limit}});
await bridge.initialize();
const input=createInterface({input:process.stdin,crlfDelay:Infinity});
let chain=Promise.resolve(),stopping=false;
async function stop(){if(stopping)return;stopping=true;await bridge.disconnect();input.close();process.stdin.destroy();setTimeout(()=>process.exit(0),20);}
input.on('line',line=>{chain=chain.then(async()=>{
  let request;
  try {
    if(Buffer.byteLength(line)>limit)throw Error('input limit');
    request=JSON.parse(line);if(request.method==='inspect'&&args[3]==='probe-fails')throw Object.assign(Error('synthetic probe refusal'),{code:'SYNTHETIC_PROBE_FAILURE'});
    const response=await bridge.handle(request);
    if(['hello','inspect','execute'].includes(request.method)&&response.ok)response.result.inputEnvelope=descriptor;
    send(response);if(request.method==='shutdown')await stop();
  } catch(error){send({protocolVersion:'director-sdk-bridge/v1',requestId:request?.requestId??null,attemptId:request?.attemptId??null,type:'response',ok:false,error:{code:error.code??'FIXTURE_ERROR',message:String(error.message)}});}
});});
input.on('close',()=>{void chain.then(stop);});
process.on('SIGTERM',()=>{void stop();});
