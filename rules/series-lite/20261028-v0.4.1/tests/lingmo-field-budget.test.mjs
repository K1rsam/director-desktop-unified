import test from 'node:test';
import assert from 'node:assert/strict';
import {splitNames,inspectFieldBudget} from '../scripts/lingmo-field-budget.mjs';

const md=props=>`**【标准化剧本】**\n第1集\n1-1\n场景：门口\n人物：甲\n道具：${props}\n`;
test('checks the full scene union, not just individual short names',()=>{
 const names=Array.from({length:21},(_,i)=>String.fromCharCode(65+i)+'A');
 const r=inspectFieldBudget(md(names.join('、')));
 assert.equal(r.ok,true);assert.equal(r.reviewRequired,true);assert.deepEqual(r.warnings[0].exceeded,['jsonArrayCharacters']);
 assert.equal(r.warnings[0].count,21);assert.equal(r.warnings[0].line,6);
});
test('escaped delimiters and backslashes preserve name identity',()=>{
 assert.deepEqual(splitNames(String.raw`杯\、盘、杆\\柄`),['杯、盘','杆\\柄']);
 const r=inspectFieldBudget(md(String.raw`杯\、盘、杆\\柄`));
 assert.equal(r.ok,true);assert.equal(r.maxima.jsonArrayCharacters,[...JSON.stringify(['杯、盘','杆\\柄'])].length);
});
test('unicode uses codepoints and bytes, not UTF16 length',()=>{
 const r=inspectFieldBudget(md('😀'.repeat(61)));
 assert.equal(r.ok,true);assert.equal(r.reviewRequired,true);assert.equal(r.maxima.characters,61);assert.equal(r.maxima.utf8Bytes,244);
});
test('catalog and quoted prose cannot change structural field totals',()=>{
 const r=inspectFieldBudget(md('AA、AB')+'  道具：'+ '长'.repeat(100)+'\n**【道具基础设定库】**\n场景：文本\n人物：文本\n道具：'+'长'.repeat(100)+'\n');
 assert.equal(r.ok,true);assert.equal(r.scenes,1);
 assert.equal(inspectFieldBudget('无场级目录').ok,false);
});

import {renderLingmoV7,verifyLingmoProjection} from '../scripts/lingmo-v7.mjs';
test('sample envelope is not a backend hard limit; missing scene data still fails',()=>{
 const props='长'.repeat(82), projected={episodes:[{number:1,scenes:[{id:'1-1',assets:['门','甲',props],units:[]}]}]};
 assert.doesNotThrow(()=>renderLingmoV7(projected));
 assert.throws(()=>renderLingmoV7({episodes:[]}),{code:'LINGMO_SCENE_PROP_BUDGET'});
 assert.throws(()=>verifyLingmoProjection(projected,'empty'),{code:'LINGMO_SCENE_PROP_BUDGET'});
});
