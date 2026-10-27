import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {checkBundle,projectBundle,renderProjection} from '../scripts/storyboard.mjs';
import {buildLingmoCatalog} from '../scripts/lingmo-v7.mjs';

const fixture=()=>JSON.parse(readFileSync(new URL('fixtures/lingmo-v7-bundle.json',import.meta.url)));
const scope=()=>({version:'series-season-scope-v1',id:'season-1',revision:1,seasonNumber:1,status:'locked',sourceHash:'a'.repeat(64),
 readingScope:[{number:1,sceneIds:['1-1','1-2']},{number:2,sceneIds:['2-1']}],episodes:[{number:1,sceneIds:['1-1','1-2']}],
 target:{minSeconds:7200,maxSeconds:9000,planningSeconds:8100},
 ending:{episode:1,sceneId:'1-2',sourceRef:'source-body.txt:L1-L8',closure:'本集原生冲突已闭合。',hook:'下一场的门外声保留悬念。',estimate:{minSeconds:7400,maxSeconds:8500}},
 confirmation:{kind:'delegated',basis:'test_fixture: 已授权选季'}});

test('proposed season cannot enter the existing storyboard/export consumer',()=>{
 const b=fixture();b.productionScope=scope();b.productionScope.status='proposed';
 assert.throws(()=>checkBundle(b),/SEASON|季|锁定|locked/);
 assert.throws(()=>projectBundle(b),/SEASON|季|锁定|locked/);
});
test('selected season cannot export a different, otherwise valid source episode',()=>{
 const b=fixture(),s=scope();s.episodes=[s.readingScope[0]];
 s.readingScope.unshift({number:0,sceneIds:['0-1']});s.episodes=[s.readingScope[0]];
 s.ending={...s.ending,episode:0,sceneId:'0-1'};b.productionScope=s;
 assert.throws(()=>checkBundle(b),/SEASON|季|范围/);
});
test('an externally bound season cannot be stripped from a bundle',()=>{
 assert.throws(()=>checkBundle(fixture(),{seasonScope:scope()}),/SEASON|季|productionScope/);
});
test('a preview is labeled partial and final delivery rejects incomplete or short coverage',()=>{
 const b=fixture();b.productionScope=scope();
 let result=checkBundle(b);assert.equal(result.season.coverage.complete,true);
 assert.equal(result.season.duration.status,'outside-target');
 b.seasonDelivery='final';assert.throws(()=>checkBundle(b),/SEASON|季|时长|120|7200/);
 const s=scope();s.episodes.push(s.readingScope[1]);s.ending={...s.ending,episode:2,sceneId:'2-1'};b.productionScope=s;
 assert.throws(()=>checkBundle(b),/SEASON|季|覆盖|缺/);
});
test('scope is carried in the image catalog without entering generation text',()=>{
 const b=fixture();b.productionScope=scope();const p=projectBundle(b),md=renderProjection(p);
 const catalog=buildLingmoCatalog(b,p,md,'b'.repeat(64));
 assert.deepEqual(catalog.productionScope,b.productionScope);assert.match(catalog.scopeHash,/^[a-f0-9]{64}$/);
 assert.doesNotMatch(md,/series-season-scope-v1|productionScope|本集原生冲突已闭合/);
 assert.equal(checkBundle(fixture()).season,undefined);
});
