import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdtemp,access,cp} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {dirname,join,resolve} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const cli=join(root,'scripts/prepare-source.mjs');
const rich='第1集\n1-1 夜 内 门厅\n人物：守门人、阿青、阿白、众人\n连续性备注：面罩不摘。\n△守门人右手提灯。\n守门人（OS）：只是风。\n守门人（VO）：\n“夜还没完，\n别松懈。”\n系统音：门锁正常。\n阿青：我（压低声音）听见了。\n阿青：开门。 阿白：别急。\n众人：好！\n字幕：次日\n';

async function run(text=rich,{script=cli,speakers=['守门人','阿青','阿白','众人'],outputName='source.json',cwd}={}){
 const directory=await mkdtemp(join(tmpdir(),'ssd-intake-test-'));
 const input=join(directory,'episode.txt'),known=join(directory,'speakers.json'),output=join(directory,outputName);
 await writeFile(input,text);await writeFile(known,JSON.stringify(speakers));
 const result=spawnSync(process.execPath,[script,'--text',input,'--key','E01','--known-speakers',known,'--output',output],{encoding:'utf8',cwd:cwd||directory});
 return {...result,directory,input,known,output};
}
const successful=result=>{assert.equal(result.status,0,result.stdout+result.stderr);return JSON.parse(result.stdout);};


test('current intake keeps cast/notes out of dialogue and retains action/subtitle typing',async()=>{
 const r=await run();const status=successful(r);const source=JSON.parse(await readFile(r.output,'utf8'));
 assert.equal(status.ok,true);assert.equal(source.sourceProtocol,'standard-episode-v1');
 assert.equal(source.standardPackage.schemaVersion,'DIRECTOR_STANDARD_SCRIPT_SLICE_V5');
 assert.deepEqual(source.dialogue.map(q=>q.speaker),['守门人','守门人','系统音','阿青','阿青','阿白','众人']);
 assert.ok(source.contextRows.some(r=>r.text.includes('连续性备注：面罩不摘。')&&r.evidenceRole==='guidance'));
 assert.ok(source.sourceRows.some(r=>r.standard.kind==='SCENE_METADATA'&&r.text.startsWith('人物：')));
 assert.ok(source.sourceRows.some(r=>r.standard.kind==='ACTION'&&r.text.includes('右手提灯')));
 assert.deepEqual(source.onscreenTexts.map(t=>t.text),['次日']);
});

test('OS VO system multiline group and inline cue preserve spoken bytes and cue coordinates',async()=>{
 const r=await run();successful(r);const source=JSON.parse(await readFile(r.output,'utf8'));
 assert.deepEqual(source.dialogue.slice(0,3).map(q=>[q.channel,q.quote]),[['OS','只是风。'],['VO','“夜还没完，\n别松懈。”'],['系统音','门锁正常。']]);
 assert.equal(source.dialogue[3].quote,'我听见了。');assert.equal(source.dialogue[3].performanceCues[0].text,'压低声音');
 assert.deepEqual(source.dialogue.slice(4).map(q=>q.quote),['开门。','别急。','好！']);
 assert.equal(source.intakeEvidence.semantic,'NOT_RUN');
 assert.equal(source.intakeEvidence.receipt.semantic,'NOT_RUN');
});

test('physical source lines and original bytes remain fully covered by compile receipt',async()=>{
 const text=rich+'\n△门保持关闭。\n';const r=await run(text);successful(r);const source=JSON.parse(await readFile(r.output,'utf8'));
 assert.equal(source.intakeEvidence.sourceText,text);
 assert.equal(source.intakeEvidence.sourceSha256,createHash('sha256').update(await readFile(r.input)).digest('hex'));
 assert.equal(source.standardPackage.evidence.map(u=>u.text).join(''),text);
 assert.equal(source.intakeEvidence.lines.map(l=>l.text).join(''),text);
 assert.equal(source.intakeEvidence.receipt.checks.unitCoverage,'VERIFIED');
 assert.equal(source.intakeEvidence.receipt.checks.sourceOrder,'VERIFIED');
 const ids=source.standardPackage.evidence.map(u=>u.id);
 assert.deepEqual(source.standardPackage.fidelity.assignments.map(a=>a.unitId),ids);
});

test('unknown heading blocks with exact diagnostic and creates no output',async()=>{
 const r=await run('第1集\n1-1 夜 内 门厅\n接口约束：不要把我当角色。\n△木门关闭。\n');
 assert.notEqual(r.status,0);const status=JSON.parse(r.stdout);assert.equal(status.ok,false);
 assert.ok(status.diagnostics.some(d=>d.code==='UNKNOWN_COLON_LINE'&&d.line===3));
 await assert.rejects(access(r.output));
});

test('unclosed or unsupported inline ownership remains diagnostics instead of guessed speech',async()=>{
 const r=await run('第1集\n1-1 夜 内 门厅\n守门人：“还没结束。\n');
 assert.notEqual(r.status,0);const status=JSON.parse(r.stdout);
 assert.ok(status.diagnostics.some(d=>d.code==='UNCLOSED_DIALOGUE_QUOTE'&&d.line===3));
 await assert.rejects(access(r.output));
});

test('no BODY and invalid known-speaker list produce no source',async()=>{
 const empty=await run('第1集\n人物：守门人\n连续性备注：别摘面罩。\n');
 assert.notEqual(empty.status,0);assert.equal(JSON.parse(empty.stdout).ok,false);await assert.rejects(access(empty.output));
 const invalid=await run(rich,{speakers:{name:'守门人'}});
 assert.notEqual(invalid.status,0);await assert.rejects(access(invalid.output));
});

test('Markdown and fullwidth/NBSP headings stay structural with exact source coordinates',async()=>{
 for(const prefix of ['# ','　　','\u00a0']){
  const text=`${prefix}第1集\r\n${prefix}1-1 夜 内 门厅\r\n△门上有😀。\r\n守门人：别碰。\r\n`;
  const r=await run(text);successful(r);const source=JSON.parse(await readFile(r.output,'utf8'));
  assert.deepEqual(source.sourceRows.slice(0,2).map(row=>[row.standard.target,row.standard.kind]),[['STRUCTURE','EPISODE_HEADING'],['STRUCTURE','SCENE_HEADING']]);
  assert.equal(source.intakeEvidence.sourceText,text);
  assert.equal(source.intakeEvidence.sourceSha256,createHash('sha256').update(await readFile(r.input)).digest('hex'));
  assert.equal(source.standardPackage.evidence.map(unit=>unit.text).join(''),text);
  assert.equal(source.intakeEvidence.receipt.checks.unitCoverage,'VERIFIED');
  assert.equal(source.intakeEvidence.structureMappings.length,2);
  const {normalizeSource}=await import(pathToFileURL(join(root,'scripts/core/runtime/storyboard-v4/core/source.mjs')));
  assert.doesNotThrow(()=>normalizeSource(source));
 }
});

test('formatting-only headings cannot satisfy the BODY requirement',async()=>{
 for(const text of ['　　第1集\n　　1-1 夜 内 门厅\n','\u00a0第1集\n\u00a01-1 夜 内 门厅\n','# 第1集\n## 1-1 夜 内 门厅\n']){
  const r=await run(text);assert.notEqual(r.status,0);assert.equal(JSON.parse(r.stdout).code,'INTAKE_NO_BODY');await assert.rejects(access(r.output));
 }
});

test('mapped headings after speech use the compiler and retain unrelated diagnostics',async()=>{
 const text='# 第1集：门厅\n## 1-1 夜 内 门厅\n守门人：停。\n## 1-2 夜 外 门口\n△门打开。\n';
 const r=await run(text);successful(r);const source=JSON.parse(await readFile(r.output,'utf8'));
 assert.equal(source.sourceRows.filter(row=>row.standard.kind==='SCENE_HEADING').length,2);
 assert.equal(source.dialogue[0].quote,'停。');
 assert.equal(source.standardPackage.evidence.map(unit=>unit.text).join(''),text);
 const bad=await run(text+'接口约束：不可当角色。\n');assert.notEqual(bad.status,0);
 assert.ok(JSON.parse(bad.stdout).diagnostics.some(d=>d.line===6&&d.code==='UNKNOWN_COLON_LINE'));
 await assert.rejects(access(bad.output));
});

test('one episode key cannot silently combine multiple episode headings',async()=>{
 for(const prefix of ['', '# ', '　　']){
  const r=await run(`${prefix}第1集\n${prefix}1-1 夜 内 门厅\n△门关闭。\n${prefix}第2集\n${prefix}2-1 日 外 门口\n△门打开。\n`);
  assert.notEqual(r.status,0);const result=JSON.parse(r.stdout);assert.equal(result.code,'INTAKE_MULTIPLE_EPISODES');
  assert.ok(result.diagnostics.some(d=>d.line===4));await assert.rejects(access(r.output));
 }
});

test('a quoted episode label remains speech rather than a second episode',async()=>{
 const text='第1集\n1-1 夜 内 门厅\n守门人：“目录写着\n第2集\n这三个字。”\n';
 const r=await run(text);successful(r);const source=JSON.parse(await readFile(r.output,'utf8'));
 assert.equal(source.dialogue[0].quote,'“目录写着\n第2集\n这三个字。”');
 assert.equal(source.sourceRows.filter(row=>row.standard.kind==='EPISODE_HEADING').length,1);
});

test('source version/text tampering is rejected by preserved original core protocol',async()=>{
 const r=await run();successful(r);const source=JSON.parse(await readFile(r.output,'utf8'));
 const {normalizeSource}=await import(pathToFileURL(join(root,'scripts/core/runtime/storyboard-v4/core/source.mjs')));
 const {standardPackageSource}=await import(pathToFileURL(join(root,'scripts/core/runtime/storyboard-v4/core/standard-source.mjs')));
 const badVersion=structuredClone(source);badVersion.standardPackage.binding.sourceVersion='src:tampered';
 assert.throws(()=>normalizeSource(badVersion));
 const badRows=structuredClone(source);badRows.sourceRows.find(r=>r.standard.kind==='ACTION').text+='删改';
 assert.throws(()=>normalizeSource(badRows));
 assert.throws(()=>standardPackageSource(source.standardPackage,{key:'E01',sourceText:rich+'改源'}));
});

test('output is exclusive and non-UTF8 source is rejected without replacement characters',async()=>{
 const r=await run();successful(r);const original=await readFile(r.output);
 const repeated=spawnSync(process.execPath,[cli,'--text',r.input,'--key','E01','--known-speakers',r.known,'--output',r.output],{encoding:'utf8'});
 assert.notEqual(repeated.status,0);assert.deepEqual(await readFile(r.output),original);
 const output=join(r.directory,'invalid-source.json');await writeFile(r.input,Buffer.from([0xff,0xfe,0x00]));
 const invalid=spawnSync(process.execPath,[cli,'--text',r.input,'--key','E01','--known-speakers',r.known,'--output',output],{encoding:'utf8'});
 assert.notEqual(invalid.status,0);await assert.rejects(access(output));
});

test('standalone copy works from unrelated cwd and symlink entry; runtime closure has no TS/npm',async()=>{
 const directory=await mkdtemp(join(tmpdir(),'ssd-intake-portable-'));const scripts=join(directory,'scripts');
 await cp(join(root,'scripts'),scripts,{recursive:true});
 const entry=join(directory,'symlink-entry.mjs');const {symlink}=await import('node:fs/promises');await symlink(join(scripts,'prepare-source.mjs'),entry);
 const r=await run(rich,{script:entry,cwd:tmpdir()});successful(r);
 const seen=new Set();async function visit(file){if(seen.has(file))return;seen.add(file);const text=await readFile(file,'utf8');for(const [,specifier] of text.matchAll(/(?:from\s*|import\s*)['"]([^'"]+)['"]/g)){if(specifier.startsWith('node:'))continue;assert.ok(specifier.startsWith('.'),specifier);assert.equal(specifier.endsWith('.ts'),false);await visit(resolve(dirname(file),specifier));}}
 await visit(join(scripts,'prepare-source.mjs'));assert.ok(seen.size>=5);
});

test('generated intake source hashes and runtime imports match checked provenance',async()=>{
 const manifest=JSON.parse(await readFile(join(root,'scripts/intake/provenance.json'),'utf8'));
 assert.match(manifest.generator.nodeVersion,/^v24\./);
 for(const item of manifest.files){const contents=await readFile(join(root,'scripts/intake',item.packaged));assert.equal(createHash('sha256').update(contents).digest('hex'),item.generatedSha256);assert.match(item.sourceSha256,/^[a-f0-9]{64}$/);}
 assert.equal(manifest.generator.mode,'transform');assert.equal(manifest.runtimeCompilationRequired,false);
});

test('CRLF and emoji keep original UTF-8 hash and exact UTF16 coordinates',async()=>{
 const text='第1集\r\n1-1 夜 内 门厅\r\n人物：守门人\r\n△门板上有😀符号。\r\n守门人：别碰😀。\r\n';
 const r=await run(text);successful(r);const source=JSON.parse(await readFile(r.output,'utf8'));
 assert.equal(source.intakeEvidence.sourceText,text);
 assert.equal(source.intakeEvidence.sourceSha256,createHash('sha256').update(await readFile(r.input)).digest('hex'));
 let offset=0;for(const unit of source.standardPackage.evidence){assert.equal(unit.start,offset);assert.equal(unit.end,offset+unit.text.length);assert.equal(text.slice(unit.start,unit.end),unit.text);assert.equal(unit.offsetUnit,'UTF16');offset=unit.end;}
 assert.equal(offset,text.length);assert.equal(source.dialogue[0].quote,'别碰😀。');
 const emoji=text.indexOf('😀');const unit=source.standardPackage.evidence.find(u=>u.start<=emoji&&u.end>emoji);
 assert.equal(text.slice(emoji,emoji+2),'😀');assert.ok(unit.text.includes('😀'));
 assert.equal(source.standardPackage.evidence.map(u=>u.text).join(''),text);
});

test('prepared source feeds unchanged two-scene facts through check export verify',async()=>{
 const bundle=JSON.parse(await readFile(join(root,'tests/fixtures/export-bundle.json'),'utf8'));
 const episode=bundle.episodes[0],oldRows=episode.source.sourceRows;
 const {normalizeSource}=await import(pathToFileURL(join(root,'scripts/core/runtime/storyboard-v4/core/source.mjs')));
 const oldQuotes=normalizeSource(episode.source).dialogue;
 const text=oldRows.map(r=>r.text).join('\n')+'\n';
 const r=await run(text,{speakers:['值守者']});successful(r);
 const source=JSON.parse(await readFile(r.output,'utf8'));
 assert.equal(source.standardPackage.evidence.map(u=>u.text).join(''),text);
 const ids=new Map(oldRows.map(old=>[old.id,source.sourceRows.find(row=>row.text===old.text+'\n').id]));
 const quoteIds=new Map(oldQuotes.map(old=>[old.id,source.dialogue.find(q=>q.speaker===old.speaker&&q.quote===old.quote&&q.channel===old.channel).id]));
 episode.source=source;
 for(const scene of episode.scenes)scene.sourceRefs=scene.sourceRefs.map(id=>ids.get(id));
 for(const unit of episode.draft.units){for(const voice of unit.voiceBindings)voice.lineId=quoteIds.get(voice.lineId);for(const shot of unit.shots){shot.sourceRefs=shot.sourceRefs.map(id=>ids.get(id));for(const part of shot.dialogueParts)part.lineId=quoteIds.get(part.lineId);}}
 const input=join(r.directory,'bundle.json'),output=join(r.directory,'output.md');await writeFile(input,JSON.stringify(bundle));
 for(const command of ['check','export','verify']){
  const args=[join(root,'scripts/storyboard.mjs'),command,'--input',input];if(command!=='check')args.push('--output',output);
  const result=spawnSync(process.execPath,args,{encoding:'utf8',cwd:tmpdir()});assert.equal(result.status,0,result.stdout+result.stderr);assert.equal(JSON.parse(result.stdout).ok,true);
 }
 const markdown=await readFile(output,'utf8');assert.ok(markdown.includes('值守者（现场对白｜画内'));assert.ok(markdown.includes('系统音（系统音｜系统音'));assert.ok(markdown.includes('2.5-6秒：'));
});
