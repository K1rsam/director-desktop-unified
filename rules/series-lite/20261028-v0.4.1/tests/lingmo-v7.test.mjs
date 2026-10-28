import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {checkBundle,projectBundle,renderProjection,verifyMarkdown} from '../scripts/storyboard.mjs';

const original=JSON.parse(readFileSync(new URL('./fixtures/export-bundle.json',import.meta.url),'utf8'));
export function fixture(){
 const b=structuredClone(original);b.exportProfile='lingmo-v7';
 b.assets.forEach((a,i)=>{a.importName=a.name.replaceAll('·','');a.placeholderOrdinal=i;});
 for(const e of b.episodes)for(const u of e.draft.units)u.imageBindings=[...u.bindings];
 return b;
}
const roundTrip=b=>{const md=renderProjection(projectBundle(b));assert.equal(verifyMarkdown(b,md).ok,true);return md;};

test('v7固定三行和原导演字段，@只在三个资产标签中',()=>{
 const md=roundTrip(fixture());
 assert.match(md,/^\[通用提示词\] /m);
 assert.match(md,/^\[场景\] 门厅@\[门厅SC\] 夜 \/ 内$/m);
 assert.match(md,/^\[本单元出场人物\] 值守者·面罩@\[值守者面罩CH\]$/m);
 assert.match(md,/分镜1｜0-2.5s｜2.5s\n机位\/运镜：/);
 assert.doesNotMatch(md,/^引用：|^片段正文：|^夜 内$|^\[主观视觉主体\]/m);
 assert.equal((md.match(/^场景：/gm)||[]).length,2);
 for(const line of md.split('\n').filter(x=>x.includes('@[')))assert.match(line,/^\[(场景|本单元道具|本单元出场人物)\] /);
});
test('v7外层向上取整，内部镜头和声窗保留小数',()=>{
 const b=fixture();b.episodes[0].draft.units[0].shots[0].duration=2.6;
 const md=roundTrip(b);assert.match(md,/时长：7秒/);assert.match(md,/分镜2｜2.6-6.1s｜3.5s/);assert.match(md,/台词：2.6–5.8秒/);
});
test('v7短尾外层4秒，不补镜或改声窗',()=>{
 const b=fixture();b.episodes[0].draft.units[1].shots[0].duration=3.2;
 const md=roundTrip(b);assert.match(md,/片段1\n时长：4秒\n\[通用提示词\][\s\S]*分镜1｜0-3.2s｜3.2s/);assert.match(md,/台词：0.2–2.8秒/);
});
test('历史v7正文Q版的null绑定仍兼容，不能作为新生产免图依据',()=>{
 const b=fixture(),u=b.episodes[0].draft.units[0];
 u.visualSubjects=[{name:'值守者Q版',domain:'subjective'}];u.visualSubjectBindings=[{name:'值守者Q版',domain:'subjective',assetKey:null}];
 u.shots[0].picture+='值守者短暂呈大头Q版，仍保留面罩。';
 const md=roundTrip(b);assert.match(md,/值守者短暂呈大头Q版，仍保留面罩。/);assert.doesNotMatch(md,/@\[值守者Q版|\[主观视觉主体\]/);
});
test('原声提示、类别、所有分段和单元出口不能因v7格式丢失',()=>{
 const b=fixture(),e=b.episodes[0],u=e.draft.units[0];
 e.source.sourceRows[1].text='值守者（画外咆哮）：吼！';
 u.shots.forEach((s,i)=>{s.dialogueParts[0].text=['吼','！'][i];s.dialogueParts[0].presentation='画外';});
 u.segments=u.shots.map((s,i)=>({title:`分段${i+1}`,unitDirection:`演出${i+1}逐字保留`,lighting:`分段光影${i+1}`,initialState:`分段入口${i+1}`,exitState:`分段出口${i+1}`,shotIds:[s.id]}));
 const md=roundTrip(b);
 for(const text of ['画外咆哮','兽吼',u.exitState,u.initialState,u.lighting,e.settings.prohibitions,...u.segments.flatMap(s=>[s.title,s.unitDirection,s.lighting,s.initialState,s.exitState])])assert.ok(md.includes(text),text);
 assert.doesNotMatch(md,/原声类别：roar/);
});
test('多行字段可独立反解且篡改任何可见字段被发现',()=>{
 const b=fixture();b.episodes[0].draft.units[0].shots[0].picture+='\n台词：此行是画面里的原始文字说明。\n  原有缩进。';
 const md=roundTrip(b);
 for(const [before,after] of [['原有缩进','丢掉缩进'],['2.5-6s','2.5-6.1s'],['@[门厅SC]','@[走廊SC]'],['面罩仍覆盖口鼻','面罩已被摘掉']])assert.throws(()=>verifyMarkdown(b,md.replace(before,after)),error=>/VERIFY/.test(error.code),before);
});
test('v7未知profile、漏图片清单或声源冒充图片均被拒绝',()=>{
 const b=fixture();b.exportProfile='lingmo-guess';assert.throws(()=>checkBundle(b),e=>e.code==='EXPORT_PROFILE');
 const c=fixture();delete c.episodes[0].draft.units[0].imageBindings;assert.throws(()=>checkBundle(c),e=>e.code==='IMAGE_BINDINGS');
 const d=fixture(),u=d.episodes[0].draft.units[0];u.participants=[];assert.throws(()=>checkBundle(d),e=>e.code==='IMAGE_BINDINGS');
});
test('原称呼中的令牌形状不能偷加未登记图片',()=>{
 const b=fixture(),old='铜钥匙PR',name='铜钥匙@[隐藏PR]',key=name+'PR',asset=b.assets.find(a=>a.key===old);
 asset.name=name;asset.key=key;
 for(const e of b.episodes){for(const scene of e.scenes)scene.bindings=scene.bindings.map(k=>k===old?key:k);for(const u of e.draft.units){u.bindings=u.bindings.map(k=>k===old?key:k);u.imageBindings=u.imageBindings.map(k=>k===old?key:k);u.props=u.props.map(n=>n==='铜钥匙'?name:n);}}
 assert.throws(()=>projectBundle(b),e=>e.code==='LINGMO_RESERVED_TOKEN');
});
test('原场号中的完整或不完整令牌形状不能偷加图片',()=>{
 for(const label of ['1-1@[未登记幽灵CH]','1-1@[']){
  const b=fixture(),e=b.episodes[0];b.scope[0].sceneIds[0]=label;e.scenes[0].id=label;e.draft.units[0].sceneId=label;
  assert.throws(()=>projectBundle(b),error=>error.code==='LINGMO_RESERVED_TOKEN'&&error.message.includes('第1集')&&error.message.includes(label));
 }
});
test('原场号不能伪造第二个标准化剧本区',()=>{
 for(const label of ['**【标准化剧本】**','  **【标准化剧本】**  ']){
  const b=fixture(),e=b.episodes[0];b.scope[0].sceneIds[0]=label;e.scenes[0].id=label;e.draft.units[0].sceneId=label;
  assert.throws(()=>projectBundle(b),error=>error.code==='LINGMO_RESERVED_SCENE'&&error.message.includes('第1集'));
 }
});
