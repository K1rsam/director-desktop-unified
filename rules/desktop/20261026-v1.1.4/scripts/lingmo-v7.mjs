import {checkVisualBaselines,composeGeneralPrompt} from './visual-baseline.mjs';
import {assertFieldBudget} from './lingmo-field-budget.mjs';
import {isDeepStrictEqual} from 'node:util';
import {createHash} from 'node:crypto';
import {normalizeSource} from './core/runtime/storyboard-v4/core/source.mjs';
import {SOUND_EVENT_PRESENTATION,validateSoundEvents,soundEventDelivery} from './core/runtime/storyboard-v4/core/sound-events.mjs';
import {SECOND_TICKS,timeTicks,timeText,containerSeconds} from './time-values.mjs';
import {importAssetFields} from './lingmo-asset-names.mjs';
import {hashSeasonScope} from './season-scope.mjs';

const need=(ok,code,message)=>{if(!ok)throw Object.assign(new Error(message),{code});};
const has=v=>typeof v==='string'&&v.trim();
const assetLabels=['场景','人物','道具'];
const assetTypes=['SC','CH','PR'];
const tokenLabels=new Set(['场景','本单元道具','本单元出场人物']);
const headerLabels=['通用提示词','场景','本单元道具','本单元出场人物','本单元光影','人物状态与站位前提'];
const extraHeaderLabels=new Set(['单元光影','单元起始状态']);
const voiceKinds={speech:'口语',roar:'兽吼',scream:'尖叫',vocalization:'非词人声'};
const hash=text=>createHash('sha256').update(text).digest('hex');
const value=(label,text)=>[label,text];
const ordered=(keys,assets)=>[...keys].sort((a,b)=>assetTypes.indexOf(assets.get(a).type)-assetTypes.indexOf(assets.get(b).type));

// Identity is supplied by checked bindings. Never resolve an asset by searching prose.
function unitHeader(e,scene,u,assets){
 const images=new Set(u.imageBindings),inserted=new Set();
 const append=(name,key)=>{
  if(!images.has(key)||inserted.has(key))return name;
  inserted.add(key);return `${name}@[${importAssetFields(assets.get(key)).token}]`;
 };
 const keyFor=(name,type)=>u.bindings.find(key=>assets.get(key).name===name&&assets.get(key).type===type);
 const characters=u.participants.map(name=>append(name,keyFor(name,'CH')));
 for(const subject of u.mediaSubjects||[]){
  const domain={screen:'屏幕中',photo:'照片或画像中',flashback:'历史插入中'}[subject.medium];
  characters.push(`${domain}：${append(subject.name,keyFor(subject.name,'CH'))}`);
 }
 for(const subject of u.visualSubjectBindings||[])if(subject.assetKey!==null&&images.has(subject.assetKey)){
  characters.push(`${subject.domain==='subjective'?'主观表现':'未来图示'}：${append(subject.name,subject.assetKey)}`);
 }
 const opening=u.segments?.[0]||u;
 const sceneNames=[u.scene,...u.bindings.filter(key=>assets.get(key).type==='SC'&&assets.get(key).name!==u.scene).map(key=>assets.get(key).name)];
 const header=[
  value('通用提示词',composeGeneralPrompt(e.settings,u.unitDirection)),
  value('场景',`${sceneNames.map(name=>append(name,keyFor(name,'SC'))).join('、')} ${(u.dayInterior??scene.dayInterior).replace(' ',' / ')}`),
  value('本单元道具',u.props.map(name=>append(name,keyFor(name,'PR'))).join('、')||'无'),
  value('本单元出场人物',characters.join('、')||'无'),
  value('本单元光影',opening.lighting),value('人物状态与站位前提',opening.initialState)
 ];
 if(opening!==u&&u.lighting!==opening.lighting)header.push(value('单元光影',u.lighting));
 if(opening!==u&&u.initialState!==opening.initialState)header.push(value('单元起始状态',u.initialState));
 need(inserted.size===images.size,'IMAGE_BINDINGS',`${u.id} 有图片未能落入其明确对应的资产标签`);
 const text=header.filter(([label])=>tokenLabels.has(label)).map(([,content])=>content).join('\n');
 const actual=[...text.matchAll(/@\[([^\]]+)\]/g)].map(m=>m[1]);
 const expected=[...images].map(key=>importAssetFields(assets.get(key)).token);
 need((text.match(/@\[/g)||[]).length===actual.length&&actual.length===expected.length&&new Set(actual).size===actual.length&&actual.every(token=>expected.includes(token)),'LINGMO_RESERVED_TOKEN',`${u.id} 资产标签中有来自原称呼的额外或不完整令牌；不能静默新增图片引用`);
 return header;
}

export function projectLingmoV7(bundle){
 checkVisualBaselines(bundle);
 const assets=new Map(bundle.assets.map(a=>[a.key,a]));
 const episodes=bundle.episodes.map(e=>{
  const source=normalizeSource(e.source);validateSoundEvents(e.draft,source);
  const quotes=new Map(source.dialogue.map(q=>[q.id,q])),spokenProgress=new Map();
  const scenes=e.scenes.map(scene=>{
   need(!scene.id.includes('@['),'LINGMO_RESERVED_TOKEN',`第${e.number}集/${scene.id} 原场号含导入令牌形状，须在采用阶段明确处理，不能静默新增图片引用或改号`);
   need(scene.id.trim()!=='**【标准化剧本】**','LINGMO_RESERVED_SCENE',`第${e.number}集/${scene.id} 原场号与唯一标准区标记冲突，须明确处理，不能静默改号或生成第二个标准区`);
   const units=e.draft.units.filter(u=>u.sceneId===scene.id);
   const images=[...new Set(units.flatMap(u=>u.imageBindings))];
   const catalog=assetTypes.map(type=>ordered(images,assets).filter(k=>assets.get(k).type===type).map(k=>assets.get(k).importName).join('、')||'无');
   return {id:scene.id,assets:catalog,units:units.map((u,index)=>{
    let cursor=0;
    const shots=u.shots.map((s,i)=>{
     const start=cursor,duration=timeTicks(s.duration,`${u.id}/${s.id}.duration`,'DURATION',true);cursor+=duration;
     const segment=u.segments?.find(segment=>segment.shotIds[0]===s.id),before=[];
     if(segment){
      const end=u.shots.findIndex(shot=>shot.id===segment.shotIds.at(-1))+1,range=`分镜${i+1}—${end}`;
      for(const [label,key] of [['分段标题','title'],['分段演出','unitDirection'],['光影承接','lighting'],['人物状态','initialState'],['分段结束状态','exitState']])if(has(segment[key]))before.push(value(`${label}｜${range}`,segment[key]));
     }
     const fields=[value('机位/运镜',s.camera),value('画面',s.picture),value('光影',s.lighting),value('声音',s.sound)];
     let humanParts=0;
     for(const p of s.dialogueParts){
      const q=quotes.get(p.lineId),from=timeText((start+timeTicks(p.voiceStart,`${s.id}/${p.lineId}.voiceStart`))/SECOND_TICKS),to=timeText((start+timeTicks(p.voiceEnd,`${s.id}/${p.lineId}.voiceEnd`))/SECOND_TICKS);
      if(p.presentation===SOUND_EVENT_PRESENTATION){fields[3][1]+='\n'+soundEventDelivery(p,from,to);continue;}
      humanParts++;
      const spokenStart=spokenProgress.get(q.id)||0,spokenEnd=spokenStart+p.text.length;
      const performance=[];
      for(const cue of q.performanceCues||[]){
       need(has(cue.text)&&Number.isInteger(cue.quoteOffset)&&cue.quoteOffset>=0&&cue.quoteOffset<=q.quote.length,'VOICE_CUE_POSITION',`${s.id}/${q.id} 句中原声提示缺少有效的原句位置`);
       // At a split boundary, attach to the following part; a trailing cue belongs
       // to the final part. Spoken text and its actual voice windows remain unchanged.
       if(cue.quoteOffset<spokenStart||cue.quoteOffset>spokenEnd||cue.quoteOffset===spokenEnd&&spokenEnd<q.quote.length)continue;
       const relative=cue.quoteOffset-spokenStart;
       const place=relative===0?'本段开口前':cue.quoteOffset===q.quote.length?'本句收尾后':`本段第${[...p.text.slice(0,relative)].length}个字符后`;
       if(bundle.generationTextVersion!=='voice-delivery-v1')performance.push(`句中原声提示（${place}）：${cue.text}`);
      }
      spokenProgress.set(q.id,spokenEnd);
      const cues=[q.channel,p.presentation,`原声类别：${voiceKinds[q.kind]}`,...(bundle.generationTextVersion!=='voice-delivery-v1'&&has(q.cue)?[`原声提示：${q.cue}`]:[]),...performance,`演法：${p.delivery}`];
      fields.push(value('台词',`${from}–${to}秒｜${q.speaker}（${cues.join('｜')}）：${p.text}`));
     }
     if(!humanParts)fields.push(value('台词','无'));
     fields.push(value('转场',s.transition));
     return {number:i+1,start:start/SECOND_TICKS,end:cursor/SECOND_TICKS,duration:duration/SECOND_TICKS,before,fields};
    });
    return {number:index+1,duration:containerSeconds(cursor),header:unitHeader(e,scene,u,assets),shots,footer:[value('结束状态',u.exitState)]};
   })};
  });
  return {number:e.number,scenes};
 });
 const projected={profile:'lingmo-v7',episodes};
 // A token-like source string must not silently become an extra platform asset reference.
 for(const e of episodes)for(const scene of e.scenes)for(const u of scene.units){
  for(const [label,text] of [...u.header,...u.shots.flatMap(s=>[...s.before,...s.fields]),...u.footer])if(!tokenLabels.has(label))need(!text.includes('@['),'LINGMO_RESERVED_TOKEN',`第${e.number}集/${scene.id}/片段${u.number}/${label} 含导入令牌形状，须在采用正文阶段明确处理，不能静默改字`);
 }
 return projected;
}

const tagLine=([label,text])=>`[${label}] ${text.replaceAll('\n','\n  ')}`;
const fieldLine=([label,text])=>`${label}：${text.replaceAll('\n','\n  ')}`;
export function renderUnitBody(unit){
 const lines=unit.header.map(tagLine);
 for(const shot of unit.shots)lines.push(...shot.before.map(tagLine),`分镜${shot.number}｜${timeText(shot.start)}-${timeText(shot.end)}s｜${timeText(shot.duration)}s`,...shot.fields.map(fieldLine));
 lines.push(...unit.footer.map(tagLine));return lines.join('\n')+'\n';
}
export function renderLingmoV7(projected){
 const parts=['**【标准化剧本】**\n'];
 for(const e of projected.episodes){parts.push(`第${e.number}集\n`);for(const scene of e.scenes){
  parts.push(`${scene.id}\n`+assetLabels.map((label,i)=>`${label}：${scene.assets[i]}\n`).join(''));
  for(const u of scene.units)parts.push(`片段${u.number}\n时长：${u.duration}秒\n`+renderUnitBody(u));
 }}
 const markdown=parts.join('');assertFieldBudget(markdown);return markdown;
}

// This reader uses the visible v7 grammar, never the renderer or an embedded expected object.
export function parseLingmoV7(markdown){
 const lines=markdown.split('\n');need(lines.pop()===''&&!markdown.includes('\r'),'VERIFY_SYNTAX','v7须为LF且末尾换行');let pos=0,coordinate='文件头';
 const peek=()=>lines[pos],take=()=>lines[pos++];
 const fail=message=>need(false,'VERIFY_SYNTAX',`${coordinate}（第${pos+1}行）${message}`);
 const expect=text=>{if(take()!==text)fail(`应为${text}`);};
 const sceneStart=()=>typeof peek()==='string'&&assetLabels.every((label,i)=>lines[pos+i+1]?.startsWith(`${label}：`));
 const continued=text=>{while(peek()?.startsWith('  ')&&!sceneStart())text+='\n'+take().slice(2);return text;};
 const tag=()=>{const match=/^\[([^\]\n]+)\] (.*)$/.exec(take());if(!match)fail('缺少正文标签');return [match[1],continued(match[2])];};
 const field=label=>{const line=take(),prefix=label+'：';if(!line?.startsWith(prefix))fail(`缺少逐镜${label}`);return [label,continued(line.slice(prefix.length))];};
 const decimal='(\\d+(?:\\.\\d{1,9})?)',shotRE=new RegExp(`^分镜(\\d+)｜${decimal}-${decimal}s｜${decimal}s$`);
 const ticks=s=>{const [a,b='']=s.split('.');return BigInt(a)*1000000000n+BigInt(b.padEnd(9,'0'));};
 expect('**【标准化剧本】**');const episodes=[];
 while(pos<lines.length){
  const match=/^第(-?\d+)集$/.exec(take());if(!match)fail('缺少集标题');const e={number:Number(match[1]),scenes:[]};episodes.push(e);coordinate=`第${e.number}集`;
  while(pos<lines.length&&(sceneStart()||!/^第-?\d+集$/.test(peek()))){
   if(!sceneStart())fail('原场后须紧跟三行资产');const scene={id:take(),assets:[],units:[]};e.scenes.push(scene);coordinate=`第${e.number}集/${scene.id}`;
   for(const label of assetLabels)scene.assets.push(field(label)[1]);
   while(!sceneStart()&&/^片段\d+$/.test(peek())){
    const number=Number(take().slice(2));coordinate=`第${e.number}集/${scene.id}/片段${number}`;
    const duration=/^时长：(\d+)秒$/.exec(take());if(!duration||+duration[1]<4||+duration[1]>30)fail('外层片长须整数4–30秒');
    const u={number,duration:Number(duration[1]),header:[],shots:[],footer:[]};scene.units.push(u);
    for(const label of headerLabels){const item=tag();if(item[0]!==label)fail(`缺少或错序的${label}`);u.header.push(item);}
    while(extraHeaderLabels.has(/^\[([^\]]+)\]/.exec(peek())?.[1]))u.header.push(tag());
    let previous=0n;
    while(pos<lines.length&&!sceneStart()&&!peek()?.startsWith('[结束状态] ')){
     coordinate=`第${e.number}集/${scene.id}/片段${number}/分镜${u.shots.length+1}`;
     const before=[];while(/^\[(分段标题|分段演出|光影承接|人物状态|分段结束状态)｜分镜\d+—\d+\] /.test(peek()))before.push(tag());
     const shot=shotRE.exec(take());if(!shot)fail('缺少逐镜时间行');
     const start=ticks(shot[2]),end=ticks(shot[3]),length=ticks(shot[4]);
     if(start!==previous||length<=0n||end-start!==length)fail('镜头时间不连续或镜长不一致');previous=end;
     const fields=['机位/运镜','画面','光影','声音'].map(field);fields.push(field('台词'));while(peek()?.startsWith('台词：'))fields.push(field('台词'));fields.push(field('转场'));
     u.shots.push({number:Number(shot[1]),start:Number(shot[2]),end:Number(shot[3]),duration:Number(shot[4]),before,fields});
    }
    coordinate=`第${e.number}集/${scene.id}/片段${number}`;
    if(!u.shots.length)fail('片段缺少镜头');const tail=tag();if(tail[0]!=='结束状态')fail('片段缺少结束状态');u.footer.push(tail);
    const outer=Math.max(4,Number((previous+999999999n)/1000000000n));if(previous>30000000000n||outer!==u.duration)fail('外层时长未按实际镜长向上取整');
   }
   if(!scene.units.length)fail('原场缺少片段');
  }
  if(!e.scenes.length)fail('本集缺少原场');
 }
 return {profile:'lingmo-v7',episodes};
}

export function verifyLingmoProjection(expected,markdown){
 assertFieldBudget(markdown);
 const actual=parseLingmoV7(markdown);
 if(!isDeepStrictEqual(actual,expected)){
  for(let ei=0;ei<Math.max(actual.episodes.length,expected.episodes.length);ei++){
   const a=actual.episodes[ei],e=expected.episodes[ei];
   if(!a||!e||a.number!==e.number)need(false,'VERIFY_MISMATCH',`第${e?.number??a?.number}集范围或顺序不一致`);
   for(let si=0;si<Math.max(a.scenes.length,e.scenes.length);si++){
    const s=e.scenes[si],t=a.scenes[si];
    if(!s||!t||s.id!==t.id||!isDeepStrictEqual(s.assets,t.assets))need(false,'VERIFY_MISMATCH',`第${e.number}集/${s?.id??t?.id} 原场或三行资产不一致`);
    for(let ui=0;ui<Math.max(s.units.length,t.units.length);ui++)if(!isDeepStrictEqual(s.units[ui],t.units[ui])){
     const expectedUnit=s.units[ui],actualUnit=t.units[ui],coordinate=`第${e.number}集/${s.id}/片段${ui+1}`;
     if(expectedUnit&&actualUnit)for(let shot=0;shot<Math.max(expectedUnit.shots.length,actualUnit.shots.length);shot++)if(!isDeepStrictEqual(expectedUnit.shots[shot],actualUnit.shots[shot]))need(false,'VERIFY_MISMATCH',`${coordinate}/分镜${shot+1} 镜窗、正文、原声或分段不一致`);
     need(false,'VERIFY_MISMATCH',`${coordinate} 片长、片头图片引用或结束状态不一致`);
    }
   }
  }
  need(false,'VERIFY_MISMATCH','v7独立反解结果与采用稿不一致');
 }
 return actual;
}

export function buildLingmoCatalog(bundle,projected,markdown,bundleSha256){
 const all=new Map(bundle.assets.map(a=>[a.key,a])),keys=new Set(bundle.episodes.flatMap(e=>e.draft.units.flatMap(u=>u.imageBindings)));
 const assets=ordered([...keys],all).map(key=>{
  const a=all.get(key);return {id:a.assetId??`fixture:${key}`,assetRevision:a.assetRevision??1,...importAssetFields(a)};
 });
 const units=[];
 for(const [ei,e] of bundle.episodes.entries())for(const [si,scene] of e.scenes.entries()){
  const originals=e.draft.units.filter(u=>u.sceneId===scene.id);
  for(const [ui,u] of projected.episodes[ei].scenes[si].units.entries()){
   const raw=renderUnitBody(u);units.push({episode:e.number,sceneId:scene.id,unitId:originals[ui].id,fragment:u.number,actualSeconds:u.shots.at(-1).end,containerSeconds:u.duration,imageTokens:originals[ui].imageBindings.map(key=>importAssetFields(all.get(key)).token),bodySha256:hash(raw),bodyWithoutTokensSha256:hash(raw.replace(/@\[[^\]]+\]/g,''))});
  }
 }
 return {version:'lingmo-v7-assets-v1',exportProfile:'lingmo-v7',testFixture:bundle.testFixture,bundleSha256,markdownSha256:hash(markdown),...(bundle.productionScope?{productionScope:bundle.productionScope,scopeHash:hashSeasonScope(bundle.productionScope),seasonDelivery:bundle.seasonDelivery??'preview'}:{}),assets,units};
}
