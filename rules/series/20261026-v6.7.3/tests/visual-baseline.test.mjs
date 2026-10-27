import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,mkdtempSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {proposeVisualBaseline,adoptVisualBaseline,applyVisualBaseline,validateVisualBaseline,checkVisualBaselines} from '../scripts/visual-baseline.mjs';
import {checkBundle,projectBundle,renderProjection,verifyMarkdown} from '../scripts/storyboard.mjs';
import {projectLingmoV7,parseLingmoV7} from '../scripts/lingmo-v7.mjs';

const fixture=()=>JSON.parse(readFileSync(new URL('./fixtures/lingmo-v7-bundle.json',import.meta.url),'utf8'));
const proposed=(preset='3D三渲二国漫')=>proposeVisualBaseline({id:'project-visual',revision:1,preset,aspectRatio:'16:9'});
const adopted=(preset)=>adoptVisualBaseline(proposed(preset),'合成fixture的已明确采用决定；非用户剧本');
const script=name=>fileURLToPath(new URL('../scripts/'+name,import.meta.url));
const clone=v=>structuredClone(v);

test('明确风格候选有可回查维度，不默认选择3D标签的渲染分支',()=>{
 const styles=['3D三渲二国漫','自然3D国漫','2D动漫','真人电影写实'];
 const candidates=styles.map(proposed);
 assert.equal(new Set(candidates.map(c=>c.visualStyle)).size,styles.length);
 for(const c of candidates){
  assert.equal(c.status,'candidate');
  assert.throws(()=>validateVisualBaseline(c),e=>e.code==='VISUAL_BASELINE');
  validateVisualBaseline(c,{allowCandidate:true});
  assert.doesNotMatch(c.visualStyle,/无BGM|无对白.*字幕/);
 }
 for(const name of ['3D国漫','赛博科幻','未知标签'])assert.throws(()=>proposed(name),e=>e.code==='VISUAL_BASELINE_PRESET');
 assert.throws(()=>proposeVisualBaseline({id:'x',preset:'自然3D国漫',aspectRatio:''}),e=>e.code==='VISUAL_BASELINE_PRESET');
});

test('自定义完整原文及项目音画政策不被预设覆写',()=>{
 const coverage={form:'木偶造型与针织服装',rendering:'手工定格摄影的连续明暗',materials:'毛线纤维哑光，木材有细纹',colorLighting:'暖黄主光与冷蓝背景分层',aspectRatio:'4:3',localExceptions:'仅已采用梦境镜头以剪纸平涂短暂切入，结束回接木偶空间'};
 const visualStyle='自定义采用文字：\n'+Object.values(coverage).join('；')+'。\n 保留原有缩进与尾空格。 ';
 const b=adoptVisualBaseline(proposeVisualBaseline({id:'custom',visualStyle,coverage}),'已给完整自定义方向');
 assert.equal(b.visualStyle,visualStyle);
 const input=fixture();input.episodes[0].settings.prohibitions='允许既定配乐；只在已采用对白镜头使用字幕，不加水印。';
 const out=applyVisualBaseline(input,b);
 assert.equal(out.episodes[0].settings.visualStyle,visualStyle);
 assert.equal(out.episodes[0].settings.prohibitions,input.episodes[0].settings.prohibitions);
 const md=renderProjection(projectBundle(out));
 assert.equal(verifyMarkdown(out,md).visualBaseline.mode,'adopted-baseline-v1');
 assert.ok(md.includes('允许既定配乐'));assert.doesNotMatch(md,/无BGM/);
});

test('应用基准仅变更两个设置字段，完整保留镜表、原声与局部幻想',()=>{
 const input=fixture(),u=input.episodes[0].draft.units[0];
 u.unitDirection+='主体的二维图解仅在本镜切入，落点回接原空间；三维Q版保持体块。';
 u.shots[0].picture+='短暂主观图解进入，再回到门厅。';
 const frozen=clone(input),out=applyVisualBaseline(input,adopted());
 assert.deepEqual(input,frozen);
 const stripped=clone(out);
 for(let i=0;i<stripped.episodes.length;i++){
  delete stripped.episodes[i].settings.visualBaseline;
  stripped.episodes[i].settings.visualStyle=input.episodes[i].settings.visualStyle;
 }
 assert.deepEqual(stripped,input);
 const parsed=parseLingmoV7(renderProjection(projectBundle(out)));
 for(const e of parsed.episodes)for(const s of e.scenes)for(const unit of s.units){
  const general=unit.header.find(([label])=>label==='通用提示词')[1];
  assert.ok(general.startsWith(out.episodes[0].settings.visualStyle+'\n'));
 }
 assert.ok(renderProjection(projectBundle(out)).includes(u.unitDirection));
});

test('check与直接v7投影均拒绝缩写设置、未采用候选和混合基准',()=>{
 const out=applyVisualBaseline(fixture(),adopted());
 out.episodes[0].settings.visualStyle='3D国漫';
 for(const fn of [checkBundle,projectLingmoV7])assert.throws(()=>fn(out),e=>e.code==='VISUAL_BASELINE');
 const candidate=fixture();candidate.episodes[0].settings.visualBaseline=proposed();
 assert.throws(()=>checkBundle(candidate),e=>e.code==='VISUAL_BASELINE');
 const mixed=applyVisualBaseline(fixture(),adopted());
 mixed.episodes.push({...clone(mixed.episodes[0]),number:99});
 delete mixed.episodes[1].settings.visualBaseline;
 assert.throws(()=>checkVisualBaselines(mixed),e=>e.code==='VISUAL_BASELINE');
 mixed.episodes[1].settings.visualBaseline=adopted('2D动漫');
 assert.throws(()=>checkVisualBaselines(mixed),e=>e.code==='VISUAL_BASELINE');
});

test('coverage不能声明不存在的依据或将标签复制成全部维度',()=>{
 const b=adopted();b.coverage.materials='这一句没有进入实际生成文本';
 assert.throws(()=>validateVisualBaseline(b),e=>e.code==='VISUAL_BASELINE');
 const label=adopted();label.visualStyle='3D国漫';
 for(const k of Object.keys(label.coverage))label.coverage[k]='3D国漫';
 assert.throws(()=>validateVisualBaseline(label),e=>e.code==='VISUAL_BASELINE');
 const shared=adopted();shared.coverage.form=shared.coverage.rendering;
 assert.doesNotThrow(()=>validateVisualBaseline(shared));
});

test('独立回读发现任意片段视觉头丢失，不能用其它片段的完整头代替',()=>{
 const out=applyVisualBaseline(fixture(),adopted()),md=renderProjection(projectBundle(out));
 assert.ok((md.match(/\[通用提示词\]/g)??[]).length>1);
 const changed=md.replace(out.episodes[0].settings.visualStyle,'同上');
 assert.throws(()=>verifyMarkdown(out,changed),e=>e.code==='VERIFY_MISMATCH');
});

test('无基准历史稿仍保真回读，但明确未验证共同视觉基准',()=>{
 const b=fixture(),original=clone(b),md=renderProjection(projectBundle(b));
 assert.equal(checkBundle(b).visualBaseline.mode,'legacy-unverified');
 assert.equal(verifyMarkdown(b,md).visualBaseline.mode,'legacy-unverified');
 assert.deepEqual(b,original);
});

test('CLI预设→采用→应用→检查/导出/独立回读，并拒绝覆盖输入和已有输出',()=>{
 const dir=mkdtempSync(join(tmpdir(),'visual-baseline-'));
 const file=name=>join(dir,name);
 const run=(name,args,ok=true)=>{
  const result=spawnSync(process.execPath,[script(name),...args],{encoding:'utf8'});
  if(ok)assert.equal(result.status,0,result.stderr);else assert.notEqual(result.status,0);
  return result;
 };
 try{
  writeFileSync(file('definition.json'),JSON.stringify({id:'integration',preset:'自然3D国漫',aspectRatio:'16:9'}));
  writeFileSync(file('bundle.json'),JSON.stringify(fixture()));
  const source=readFileSync(file('bundle.json'));
  run('visual-baseline.mjs',['propose','--input',file('definition.json'),'--output',file('candidate.json')]);
  run('visual-baseline.mjs',['adopt','--input',file('candidate.json'),'--basis','合成集成测试的既有采用依据','--output',file('baseline.json')]);
  run('visual-baseline.mjs',['validate','--input',file('baseline.json')]);
  run('visual-baseline.mjs',['apply','--baseline',file('baseline.json'),'--bundle',file('bundle.json'),'--output',file('updated.json')]);
  assert.deepEqual(readFileSync(file('bundle.json')),source);
  const check=run('storyboard.mjs',['check','--input',file('updated.json')]);
  assert.equal(JSON.parse(check.stdout).visualBaseline.mode,'adopted-baseline-v1');
  run('storyboard.mjs',['export','--input',file('updated.json'),'--output',file('preview.md')]);
  run('storyboard.mjs',['verify','--input',file('updated.json'),'--output',file('preview.md')]);
  run('visual-baseline.mjs',['apply','--baseline',file('baseline.json'),'--bundle',file('bundle.json'),'--output',file('bundle.json')],false);
  run('visual-baseline.mjs',['apply','--baseline',file('baseline.json'),'--bundle',file('bundle.json'),'--output',file('updated.json')],false);
  assert.deepEqual(readFileSync(file('bundle.json')),source);
 }finally{rmSync(dir,{recursive:true,force:true});}
});
