import assert from 'node:assert/strict';
import React from 'react';
import {renderToStaticMarkup} from 'react-dom/server';
import {createServer} from 'vite';

globalThis.location={search:'?preview=1'};
let networkRequests=0,mutations=0;
globalThis.fetch=async()=>{networkRequests++;throw new Error('Network is forbidden in task UI verification');};
const base={project:{id:'offline',title:'Offline'},assets:[],units:[],jobs:[],media:[],rules:[],settings:{},providers:[]};
const local={id:'local-preparation',kind:'video',provider:'dreamina',model:'seedance2.5',status:'ready',snapshot:{autoSubmit:false}};
const cases=[];
const server=await createServer({logLevel:'error',optimizeDeps:{noDiscovery:true},server:{middlewareMode:true,hmr:false},appType:'custom'});
try{
 const {TasksPanel}=await server.ssrLoadModule('/src/Panels.tsx');
 const render=job=>renderToStaticMarkup(React.createElement(TasksPanel,{workspace:{...base,jobs:[job]},writable:true,mutate:async()=>{mutations++;throw new Error('Mutation is forbidden');},inspect:()=>{}}));
 const test=(name,run)=>{run();cases.push({name,passed:true});};
 test('Local ready materials are visibly not submitted and expose no cloud recovery actions',()=>{const html=render(local);for(const text of ['本地素材就绪 · 未生成','本地素材准备','未提交，无云端任务','尚未生成','启动、刷新和恢复不会自动生成视频','素材准备','视频生成'])assert.ok(html.includes(text),text);for(const text of ['查询原任务','恢复下载','实际费用','尚未取得'])assert.ok(!html.includes(text),text);assert.ok(html.includes('完整记录'));});
 test('Known cloud identities remain real generation tasks with recovery actions',()=>{const html=render({...local,status:'processing',providerTaskId:'original-cloud-id'});for(const text of ['original-cloud-id','查询原任务','恢复下载','实际费用'])assert.ok(html.includes(text),text);assert.ok(!html.includes('未提交，无云端任务'));assert.ok(!html.includes('仅保存本地素材包'));});
 test('An unknown submit can never be relabeled as an unsubmitted local package',()=>{const html=render({...local,status:'submission_unknown',submissionUnknown:true});assert.ok(html.includes('提交结果尚未确认'));assert.ok(html.includes('核对任务回执'));assert.ok(!html.includes('未提交，无云端任务'));assert.ok(!html.includes('本地素材就绪 · 未生成'));});
 test('An implicit or enabled submit flag remains conservative',()=>{for(const snapshot of [{},{autoSubmit:true},{autoSubmit:'false'}]){const html=render({...local,snapshot});assert.ok(!html.includes('未提交，无云端任务'));assert.ok(html.includes('查询原任务'));}});
 test('Image generation keeps its existing task and recovery controls',()=>{const html=render({...local,kind:'image',status:'download_failed',providerTaskId:'original-image-id'});assert.ok(html.includes('恢复下载'));assert.ok(!html.includes('仅保存本地素材包'));});
 test('A local failure is identified as material preparation failure without a generation result',()=>{const html=render({...local,status:'failed',error:'本地素材准备结果无法保存'});assert.ok(html.includes('素材准备失败 · 未生成'));assert.ok(html.includes('本地素材准备结果无法保存'));assert.ok(!html.includes('恢复下载'));});
}finally{await server.close();}
assert.equal(networkRequests,0);assert.equal(mutations,0);
console.log(JSON.stringify({method:'Actual React TasksPanel rendering; no database or channel access',passed:cases.length,networkRequests,mutations,cases},null,2));
