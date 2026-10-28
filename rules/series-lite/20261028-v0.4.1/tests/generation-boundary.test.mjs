import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {checkGenerationBoundary} from '../scripts/generation-boundary.mjs';
import {checkBundle,projectBundle,renderProjection,verifyMarkdown} from '../scripts/storyboard.mjs';
const fixture=()=>JSON.parse(readFileSync(new URL('fixtures/lingmo-v7-bundle.json',import.meta.url)));
const active=()=>({...fixture(),generationBoundaryVersion:'video-prompt-v1'});
const bad='后期时间卡“同日夜·雨停”在左上空窗区，米白细字。';

test('声明启用；旧稿兼容；未知声明拒绝',()=>{
 const b=fixture();b.episodes[0].draft.units[0].shots[0].picture=bad;
 assert.doesNotThrow(()=>checkBundle(b));
 b.generationBoundaryVersion='unknown';assert.throws(()=>checkBundle(b),{code:'GENERATION_BOUNDARY_VERSION'});
 assert.doesNotThrow(()=>checkBundle(active()));
});
test('check、export投影与verify共用边界，并报告集/单元/镜头/字段',()=>{
 const b=active(),md=renderProjection(projectBundle(b));
 b.episodes[0].draft.units[0].shots[0].picture=bad;
 for(const call of [()=>checkBundle(b),()=>projectBundle(b),()=>verifyMarkdown(b,md)]){
  assert.throws(call,error=>error.code==='GENERATION_BOUNDARY'&&/第1集.*U1.*S1.*picture/u.test(error.message));
 }
});
test('只检查明确生成字段，包括分段演出与原声delivery',()=>{
 for(const field of ['unitDirection','lighting','initialState','exitState']){
  const b=active();b.episodes[0].draft.units[0][field]='后期添加白色字幕';assert.throws(()=>checkGenerationBoundary(b),{code:'GENERATION_BOUNDARY'});
  const c=active();c.episodes[0].draft.units[0].segments=[{[field]:'字幕字号设为24，字幕排版居中'}];assert.throws(()=>checkGenerationBoundary(c),{code:'GENERATION_BOUNDARY'});
 }
 for(const field of ['camera','picture','lighting','sound','transition']){
  const b=active();b.episodes[0].draft.units[0].shots[0][field]=bad;assert.throws(()=>checkGenerationBoundary(b),{code:'GENERATION_BOUNDARY'});
 }
 for(const field of ['visualStyle','prohibitions']){
  const b=active();b.episodes[0].settings[field]='请后期添加字幕';assert.throws(()=>checkGenerationBoundary(b),{code:'GENERATION_BOUNDARY'});
 }
 const b=active();b.episodes[0].draft.units[0].shots[0].dialogueParts[0].delivery='设置字幕字号为24';assert.throws(()=>checkGenerationBoundary(b),{code:'GENERATION_BOUNDARY'});
});
test('剧情提及、真实牌匾屏幕和禁止项不误拦',()=>{
 for(const text of ['他询问后期制作进度。','后期字幕组来电，他接起电话。','牌匾写着“后期添加字幕”。','屏幕显示：后期添加字幕。','系统界面显示“字幕字号排版设置”。','禁止后期添加字幕；不要修改字幕字号。','无额外字幕、配乐、水印。']){
  const b=active();b.episodes[0].draft.units[0].shots[0].picture=text;assert.doesNotThrow(()=>checkBundle(b),text);
 }
});
test('源内后期备注、原话与正式素材名不扫描；检查不修改任何输入',()=>{
 const b=active();b.episodes[0].source.sourceRows[0].text=bad;
 b.episodes[0].source.contextRows.push({id:'note',text:'后期添加字幕'});
 b.assets[0].name='字幕字号排版';b.episodes[0].draft.units[0].shots[0].dialogueParts[0].text='后期添加字幕';
 const before=structuredClone(b);assert.doesNotThrow(()=>checkGenerationBoundary(b));assert.deepEqual(b,before);
 const clean=active(),snapshot=structuredClone(clean);const md=renderProjection(projectBundle(clean));verifyMarkdown(clean,md);assert.deepEqual(clean,snapshot);
 const failed=active();failed.episodes[0].draft.units[0].shots[0].picture=bad;const original=structuredClone(failed);assert.throws(()=>checkBundle(failed));assert.deepEqual(failed,original);
});
