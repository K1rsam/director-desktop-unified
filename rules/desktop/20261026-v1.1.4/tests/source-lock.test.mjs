import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,mkdtempSync,rmSync,existsSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {checkBundle,projectBundle,renderProjection} from '../scripts/storyboard.mjs';
import {prepareSource} from '../scripts/prepare-source.mjs';
const base=fileURLToPath(new URL('..',import.meta.url));
const fixture=JSON.parse(readFileSync(join(base,'tests/fixtures/export-bundle.json'),'utf8'));
const cli=args=>spawnSync(process.execPath,[join(base,'scripts/storyboard.mjs'),...args],{encoding:'utf8'});
const hash=v=>createHash('sha256').update(JSON.stringify(v)).digest('hex');
const freeze=b=>({version:'series-source-lock-v1',sourceHash:'a'.repeat(64),episodes:b.episodes.map(({number,source})=>({number,source:structuredClone(source)}))});
const setup=()=>{const b=structuredClone(fixture),lock=freeze(b);b.sourceLockHash=hash(lock);b.generationTextVersion="voice-delivery-v1";return {b,lock};};
test('独立冻结源与分镜闭合；未声明锁的旧稿仍可回查且明确证据边界',()=>{
 const {b,lock}=setup();assert.equal(checkBundle(b,{sourceLock:lock}).sourceFidelity.mode,'locked-source-v1');
 assert.equal(checkBundle(fixture).sourceFidelity.mode,'bundle-only');
});
test('源行与分镜一起改词也不能绕过独立原文',()=>{
 const {b,lock}=setup(),e=b.episodes[0];
 e.source.sourceRows[1].text=e.source.sourceRows[1].text.replace('门还没开','门尚未开');
 e.draft.units[0].shots[0].dialogueParts[0].text=e.draft.units[0].shots[0].dialogueParts[0].text.replace('门还没开','门尚未开');
 assert.throws(()=>checkBundle(b,{sourceLock:lock}),/SOURCE_LOCK_SOURCE|冻结原文/);
});
for(const [label,mutate] of [
 ['标点',e=>e.source.sourceRows[1].text=e.source.sourceRows[1].text.replace('，','。')],
 ['说者',e=>e.source.sourceRows[1].text=e.source.sourceRows[1].text.replace('值守者：','路人：')],
 ['声道',e=>e.source.sourceRows[1].text=e.source.sourceRows[1].text.replace('值守者：','值守者（OS）：')],
 ['上下文',e=>e.source.contextRows=[{id:'E01:C1',text:'△新增假前情。'}]]
])test(`冻结源拒绝${label}漂移`,()=>{const {b,lock}=setup();mutate(b.episodes[0]);assert.throws(()=>checkBundle(b,{sourceLock:lock}),/冻结原文/);});
test('声明锁后缺文件、错hash或缺集均拒绝，不静默降级',()=>{
 const {b,lock}=setup();assert.throws(()=>checkBundle(b),/冻结原文|source-lock/);
 const changed=structuredClone(lock);changed.episodes[0].source.sourceRows[1].text+='！';assert.throws(()=>checkBundle(b,{sourceLock:changed}),/hash/);
 const short=structuredClone(lock);short.episodes=[];b.sourceLockHash=hash(short);assert.throws(()=>checkBundle(b,{sourceLock:short}),/范围|集/);
});
test('全剧锁可复用到保持源序的单集，不需要创作后重建锁',()=>{
 const {b,lock}=setup();lock.episodes.push({...structuredClone(lock.episodes[0]),number:0});b.sourceLockHash=hash(lock);
 assert.equal(checkBundle(b,{sourceLock:lock}).ok,true);
 b.scope[0].number=0;b.episodes[0].number=0;
 assert.equal(checkBundle(b,{sourceLock:lock}).ok,true);
 b.sourceHash='b'.repeat(64);assert.throws(()=>checkBundle(b,{sourceLock:lock}),/hash/);
});
test('freeze-source仅冻结接入稿并拒绝覆盖，check/export/verify均要求同一源锁',()=>{
 const dir=mkdtempSync(join(tmpdir(),'source-lock-'));
 try{
  const source=join(dir,'intake.json'),lockPath=join(dir,'source-lock.json'),input=join(dir,'bundle.json'),out=join(dir,'preview.md');
  writeFileSync(source,JSON.stringify({sourceHash:'a'.repeat(64),episodes:fixture.episodes.map(({number,source})=>({number,source}))}));
  let r=cli(['freeze-source','--input',source,'--output',lockPath]);assert.equal(r.status,0,r.stderr);assert.equal(existsSync(lockPath),true);
  const lock=JSON.parse(readFileSync(lockPath));const b=structuredClone(fixture);b.sourceLockHash=JSON.parse(r.stdout).sourceLockHash;b.generationTextVersion="voice-delivery-v1";assert.equal(b.sourceLockHash,hash(lock));writeFileSync(input,JSON.stringify(b));
  r=cli(['freeze-source','--input',source,'--output',lockPath]);assert.notEqual(r.status,0);
  for(const command of ['check','export','verify']){
   const args=[command,'--input',input,...(command==='check'?[]:['--output',out])];
   r=cli(args);assert.notEqual(r.status,0,command+' missing source lock must fail');
   r=cli([...args,'--source-lock',lockPath]);assert.equal(r.status,0,r.stderr);
  }
  const changed=JSON.parse(readFileSync(input));changed.episodes[0].source.sourceRows[1].text+='！';writeFileSync(input,JSON.stringify(changed));
  r=cli(['verify','--input',input,'--output',out,'--source-lock',lockPath]);assert.notEqual(r.status,0);assert.match(r.stderr,/SOURCE_LOCK_SOURCE/);
 }finally{rmSync(dir,{recursive:true,force:true});}
});
test('独立后期资料不会被投影进生成正文；场内牌匾和原声提到人名条仍保留',()=>{
 const b=structuredClone(fixture),e=b.episodes[0];b.generationTextVersion='voice-delivery-v1';
 b.postproductionNotes=[{kind:'identity_caption',text:'阿岚／守门人',instruction:'后期添加人名条阿岚'}];
 e.source.sourceRows[1].text='值守者：这张人名条，先别放上去。';
 e.draft.units[0].shots[0].dialogueParts[0].text='这张人名条，';
 e.draft.units[0].shots[1].dialogueParts[0].text='先别放上去。';
 e.draft.units[0].shots[0].picture+='门框铁牌写着“北门”。';
 const md=renderProjection(projectBundle(b));assert.doesNotMatch(md,/后期添加人名条阿岚|阿岚／守门人/);assert.match(md,/北门/);assert.match(md,/这张人名条，/);assert.match(md,/先别放上去。/);
});
for(const profile of ['export-bundle.json','lingmo-v7-bundle.json'])test(`新生成文本不自动注入制作性源cue：${profile}`,()=>{
 const b=JSON.parse(readFileSync(join(base,'tests/fixtures',profile),'utf8'));
 b.episodes[0].source.sourceRows[1].text='值守者（后期添加人名条“阿岚／守门人”）：门还没开，先别摘下面罩。';
 const lock=freeze(b);b.sourceLockHash=hash(lock);b.generationTextVersion='voice-delivery-v1';
 const md=renderProjection(projectBundle(b,{sourceLock:lock}));
 assert.doesNotMatch(md,/后期添加人名条|阿岚／守门人/);
 assert.match(md,/门还没开，/);assert.match(md,/先别摘下面罩。/);
 assert.equal(lock.episodes[0].source.sourceRows[1].text.includes('后期添加人名条'),true);
 assert.match(md,new RegExp(b.episodes[0].draft.units[0].shots[0].dialogueParts[0].delivery));
});
test('新冻结源稿不能省略生成文本策略后回退旧的原始cue注入',()=>{
 const {b,lock}=setup();delete b.generationTextVersion;
 assert.throws(()=>checkBundle(b,{sourceLock:lock}),/生成文本策略/);
});
test('标准接入的句中提示保留在冻结源，真实表演在正确分片由作者落实，后期cue不注入',()=>{
 const b=JSON.parse(readFileSync(join(base,'tests/fixtures/lingmo-v7-bundle.json'),'utf8')),e=b.episodes[0];
 const original=e.source.sourceRows,spoken=e.draft.units[0].shots;
 const raw=original.map((r,i)=>i===1?'值守者（后期添加人名条“阿岚／守门人”）：门还没开，（压低声音）先别摘下面罩。':r.text).join('\n');
 const source=prepareSource(raw,'E01',['值守者']);
 const refs=new Map(original.map((r,i)=>[r.id,source.sourceRows[i].id]));
 const ids=new Map([['E01:R2:D1',source.dialogue[0].id],['E01:R4:D2',source.dialogue[1].id]]);
 for(const scene of e.scenes)scene.sourceRefs=scene.sourceRefs.map(r=>refs.get(r));
 for(const unit of e.draft.units){
  for(const voice of unit.voiceBindings)voice.lineId=ids.get(voice.lineId);
  for(const shot of unit.shots){shot.sourceRefs=shot.sourceRefs.map(r=>refs.get(r));for(const p of shot.dialogueParts)p.lineId=ids.get(p.lineId);}
 }
 e.source=source;spoken[0].dialogueParts[0].delivery='清楚说出提醒';spoken[1].dialogueParts[0].delivery='在先别开口前压低声音，接完提醒';
 const lock=freeze(b);b.sourceLockHash=hash(lock);b.generationTextVersion='voice-delivery-v1';
 assert.equal(source.dialogue[0].cue,'后期添加人名条“阿岚／守门人”');
 assert.deepEqual(source.dialogue[0].performanceCues.map(c=>[c.text,c.quoteOffset]),[['压低声音',5]]);
 const md=renderProjection(projectBundle(b,{sourceLock:lock}));
 assert.match(md,/清楚说出提醒/);assert.match(md,/在先别开口前压低声音，接完提醒/);
 assert.doesNotMatch(md,/后期添加人名条|阿岚／守门人|句中原声提示/);
});
