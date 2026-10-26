import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { Codex } from '@openai/codex-sdk';
import { pinned } from '../scripts/package-readiness.mjs';

test('genuine pinned SDK creates a schema under Windows long TEMP before any runtime invocation', {skip:process.platform!=='win32'},async t=>{
  const sdkFile=fileURLToPath(new URL('../node_modules/@openai/codex-sdk/dist/index.js',import.meta.url));
  assert.equal(createHash('sha256').update(fs.readFileSync(sdkFile)).digest('hex'),pinned.implementation);
  const parent=fs.realpathSync(os.tmpdir());const root=fs.mkdtempSync(path.join(parent,'director-visual-long-'));
  const work=path.join(root,'a'.repeat(90),'b'.repeat(90),'c'.repeat(70),'tmp');fs.mkdirSync(work,{recursive:true});
  assert.ok(work.length>260);
  const old=new Map(['TEMP','TMP','TMPDIR'].map(k=>[k,process.env[k]]));
  t.after(()=>{
    for(const [key,value]of old){if(value===undefined)delete process.env[key];else process.env[key]=value;}
    const resolved=fs.realpathSync(root);
    assert.equal(path.dirname(resolved),parent);assert.ok(path.basename(resolved).startsWith('director-visual-long-'));
    fs.rmSync(resolved,{recursive:true,force:true});
  });
  let calls=0;
  function thread(){const thread=new Codex({codexPathOverride:process.execPath,env:{}}).startThread({model:'gpt-6.1-sol'});
    // Only replace the transport. The genuine SDK still creates/cleans its
    // schema and normalizes local_image inputs. No executable or model starts.
    thread._exec={async *run(args){calls++;assert.equal(path.toNamespacedPath(fs.realpathSync.native(path.dirname(path.dirname(args.outputSchemaFile)))),path.toNamespacedPath(fs.realpathSync.native(work)));assert.deepEqual(JSON.parse(fs.readFileSync(args.outputSchemaFile,'utf8')),{type:'object'});assert.equal(args.images.length,1);yield JSON.stringify({type:'thread.started',thread_id:'offline'});yield JSON.stringify({type:'turn.started'});yield JSON.stringify({type:'item.completed',item:{type:'agent_message',text:'{}'}});yield JSON.stringify({type:'turn.completed',usage:{input_tokens:0,output_tokens:0,cached_input_tokens:0}});}};return thread;
  }
  for(const key of old.keys())process.env[key]=work;
  let ordinaryError=null;try{const turn=await thread().runStreamed([{type:'text',text:'offline'},{type:'local_image',path:path.join(root,'no-read.png')}],{outputSchema:{type:'object'}});for await(const _ of turn.events){}}catch(error){ordinaryError=error;}
  if(ordinaryError){assert.equal(ordinaryError.code,'ENOENT');assert.equal(ordinaryError.syscall,'mkdtemp');assert.equal(calls,0);}
  const before=calls;for(const key of old.keys())process.env[key]=path.toNamespacedPath(work);
  const turn=await thread().runStreamed([{type:'text',text:'offline'},{type:'local_image',path:path.join(root,'no-read.png')}],{outputSchema:{type:'object'}});for await(const _ of turn.events){}
  assert.equal(calls,before+1);assert.deepEqual(fs.readdirSync(work),[]);
  t.diagnostic(JSON.stringify({node:process.version,parentLength:work.length,ordinary:ordinaryError?.code??'supported',namespaced:'passed',externalRuntimeCalls:0}));
});
