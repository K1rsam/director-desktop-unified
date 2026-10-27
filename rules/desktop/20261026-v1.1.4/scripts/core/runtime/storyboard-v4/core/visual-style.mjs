// A project label selects a visual basis; it must not erase that basis at the web boundary.
export const SEEDANCE_EXPORT_VERSION='seedance-execution-v3';
export const SUPPORTED_SEEDANCE_EXPORT_VERSIONS=Object.freeze(['seedance-execution-v1','seedance-execution-v2',SEEDANCE_EXPORT_VERSION]);
const presets={
 '3D国漫':'三渲二国漫；C4D/Octane风格精细渲染，赛璐璐明暗色块结合PBR材质；国漫造型与干净轮廓，发丝、布料褶皱和物体纹理清楚，明暗有层次、饱和度适中',
 '3D三渲二国漫':'C4D/Octane风格精细渲染，赛璐璐明暗色块结合PBR材质；国漫造型与干净轮廓，发丝、布料褶皱和物体纹理清楚，明暗有层次、饱和度适中',
 '2D动漫':'清晰稳定的角色线稿、分层色块与二维动画明暗；轮廓、造型比例和线条粗细连贯，前中后景有层次',
 '真人电影写实':'电影写实摄影与自然人物比例；肤质、衣料、道具保留真实质感，光线方向、接触阴影和景深清楚',
 '古装电影写实':'古装实拍质感与自然人物比例；服化、器物与环境材质细节清楚，主光、反射和景深组织空间层次',
 '赛博科幻':'未来空间的材质、光源与前中后景清楚；霓虹和发光界面照清人物与关键动作，环境材质和光色连续'
};

// Explicit visual-only candidates for independently consumed clips. This entry
// does not inherit the legacy helper's audio/subtitle defaults or choose a
// rendering branch for the ambiguous label "3D国漫".
const visualBases={
 '3D三渲二国漫':{
  form:'风格化三维建模与国漫比例，人物脸型、发束、服装轮廓及空间体块清楚稳定',
  rendering:'三渲二着色，赛璐璐分层明暗主导人物和环境观感，避免摄影级皮肤与写实CG偏移',
  materials:'PBR用于区分布料、皮革、金属和石材的粗糙度与反射；皮肤高光克制，避免统一塑料泛亮',
  colorLighting:'色彩关系统一、饱和度适中，主光方向清楚，亮面、暗面及接触阴影形成可读层次'
 },
 '自然3D国漫':{
  form:'自然三维国漫造型，人物比例、骨相、发束与服装结构连贯，场景体块和动作接触清楚',
  rendering:'柔和连续的三维明暗塑形，风格化CG表面细节，不使用赛璐璐硬切色块或实拍毛孔观感',
  materials:'布料纤维、皮革、金属与石材有不同粗糙度和反射响应，肤质柔和且不过度镜面泛亮',
  colorLighting:'统一色彩与冷暖关系，主光及反射光构成自然层次，暗部可读，避免过曝和无依据的浓重滤镜'
 },
 '2D动漫':{
  form:'清晰稳定的二维角色线稿，脸型、造型比例与线条粗细连贯，背景空间关系可读',
  rendering:'分层平涂色块与二维动画明暗，不将现实部分改成三维写实渲染',
  materials:'以纹理笔触、色块与高光形状区分衣料、金属、石材等表面，不强加写实微表面',
  colorLighting:'统一色板与明暗层级，人物从背景清楚分离，前中后景有层次而不过度虚化'
 },
 '真人电影写实':{
  form:'电影写实人物与自然比例，脸型、年龄、发型和服装结构稳定，人物与实景尺度一致',
  rendering:'电影摄影的连续明暗和真实光学质感，不使用动漫描边、赛璐璐色块或塑料CG皮肤',
  materials:'肤质、衣料、皮革、金属与石材保留各自纹理和真实反射，接触处有可信阴影',
  colorLighting:'统一电影色彩关系，光线方向明确，以主辅光、反射和适度景深组织空间，保留肤色与暗部细节'
 }
};
export const VISUAL_BASIS_PRESETS=Object.freeze(Object.keys(visualBases));
export function buildVisualBasis({style,aspectRatio,localExceptions,legacy=false}={}){
 if(legacy){
  const selected=typeof style==='string'&&style.trim()?style.trim():'3D国漫';
  const ratio=typeof aspectRatio==='string'&&aspectRatio.trim()?aspectRatio.trim():'16:9';
  return `项目风格：${selected}；画幅${ratio}；${presets[selected]||'保持上述画风的线条、材质和质感'}`;
 }
 if(!Object.hasOwn(visualBases,style))throw Object.assign(new Error(`风格标签尚未明确或无匹配预设：${style??''}；请选择 ${VISUAL_BASIS_PRESETS.join('、')}，或提供完整自定义基准`),{code:'VISUAL_BASELINE_PRESET'});
 if(typeof aspectRatio!=='string'||!/^[1-9]\d*:[1-9]\d*$/.test(aspectRatio))throw Object.assign(new Error('候选需要已选画幅，例如16:9；不自动替换项目画幅'),{code:'VISUAL_BASELINE_PRESET'});
 const coverage={...visualBases[style],aspectRatio:`画幅${aspectRatio}`,
  localExceptions:localExceptions??'局部二维平涂、图解或Q版仅按已采用镜头的主体、起止与回接执行；三维Q版不自动转二维，其余现实画面保持项目主画风'};
 return {visualStyle:[`项目视觉基准：${style}`,coverage.aspectRatio,coverage.form,coverage.rendering,coverage.materials,coverage.colorLighting,coverage.localExceptions].join('。')+'。',coverage};
}

export function buildVisualStyle({style,aspectRatio}={}){
 return [
  buildVisualBasis({style,aspectRatio,legacy:true}),
  '正常比例人物的身份、年龄气质、脸型与五官结构稳定，衣发道具连续；有力表情通过眉眼、口角、咬肌与头颈呈现，大幅身体动作保持明确支点、重心转移与接触关系',
  '制动或转体顺势接入下一动作，衣发与物件按惯性错时跟随、回摆，在动作接续中继续回落',
  '摄影随动作、视线和结果变化组织观看，主体清楚；人物与空间受光方向明确，明暗层次连贯',
  '画内说者口型同步，画外/OS/VO/系统音依台词标记呈现，听者保持自己的反应；台词清楚，动作音与环境声衬托发声',
  '无对白、OS、旁白字幕条，无BGM、水印及额外logo；必要屏幕信息保留'
 ].join('。')+'。';
}
