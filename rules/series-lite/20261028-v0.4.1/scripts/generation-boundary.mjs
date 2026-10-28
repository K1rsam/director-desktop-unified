// Opt-in, conservative editorial-instruction checks. This is not a semantic review.
// Source, dialogue text, names and source cues are deliberately never traversed.
const list=value=>Array.isArray(value)?value:[];
const fail=(code,message)=>{throw Object.assign(new Error(message),{code});};
const performance=['unitDirection','lighting','initialState','exitState'];
const shotFields=['camera','picture','lighting','sound','transition'];

function editorialInstruction(value){
 if(typeof value!=='string')return false;
 // Quoted diegetic text remains literal content, including signs and system screens.
 const unquoted=value.replace(/“[^”]*”|「[^」]*」|『[^』]*』|"[^"\n]*"/gu,'〔原样文字〕');
 return unquoted.split(/[。！？；;\n，,]/u).some(part=>{
  const clause=part.trim().replace(/^(?:请|需要|需|须|应|建议)\s*/u,'');
  // Anchor to an instruction clause: mentioning a post-production job in a story
  // (e.g. 他询问后期制作进度) must not match a keyword search.
  return /^(?:后期(?:时间卡|字幕|叠字|标题卡)\s*(?:〔|放|置|位于|在|采用|使用|：|:)|后期(?:添加|加上|加入|叠加|制作|插入)(?:[^。！？；;\n]{0,24})(?:字幕|时间卡|标题卡|叠字|文字))/u.test(clause)
   || /^(?:字幕(?:的)?(?:字号|字体|排版|字距|行距)|(?:调整|设置|修改)字幕(?:的)?(?:字号|字体|排版|字距|行距))/u.test(clause);
 });
}

export function checkGenerationBoundary(bundle){
 const version=bundle?.generationBoundaryVersion;
 if(version===undefined)return {mode:'legacy'};
 if(version!=='video-prompt-v1')fail('GENERATION_BOUNDARY_VERSION','未知生成正文边界策略');
 const check=(value,path)=>{
  if(editorialInstruction(value))fail('GENERATION_BOUNDARY',`${path} 含后期字幕/时间卡或文字排版指令；请将编辑说明移到独立后期交接资料，生成正文保留镜内动作与声画，不自动删改源或台词`);
 };
 for(const [ei,episode] of list(bundle.episodes).entries()){
  const ep=`episodes[${ei}]（第${episode.number}集）`;
  for(const key of ['visualStyle','prohibitions'])check(episode.settings?.[key],`${ep}.settings.${key}`);
  for(const [ui,unit] of list(episode.draft?.units).entries()){
   const up=`${ep}.draft.units[${ui}]（${unit.id}）`;
   for(const key of performance)check(unit[key],`${up}.${key}`);
   for(const [si,segment] of list(unit.segments).entries())for(const key of performance)check(segment[key],`${up}.segments[${si}].${key}`);
   for(const [si,shot] of list(unit.shots).entries()){
    const sp=`${up}.shots[${si}]（${shot.id}）`;
    for(const key of shotFields)check(shot[key],`${sp}.${key}`);
    for(const [pi,part] of list(shot.dialogueParts).entries())check(part.delivery,`${sp}.dialogueParts[${pi}].delivery`);
   }
  }
 }
 return {mode:version};
}
