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

export function buildVisualStyle({style,aspectRatio}={}){
 const selected=typeof style==='string'&&style.trim()?style.trim():'3D国漫';
 const ratio=typeof aspectRatio==='string'&&aspectRatio.trim()?aspectRatio.trim():'16:9';
 const basis=presets[selected]||'保持上述画风的线条、材质和质感';
 return [
  `项目风格：${selected}；画幅${ratio}；${basis}`,
  '正常比例人物的身份、年龄气质、脸型与五官结构稳定，衣发道具连续；有力表情通过眉眼、口角、咬肌与头颈呈现，大幅身体动作保持明确支点、重心转移与接触关系',
  '制动或转体顺势接入下一动作，衣发与物件按惯性错时跟随、回摆，在动作接续中继续回落',
  '摄影随动作、视线和结果变化组织观看，主体清楚；人物与空间受光方向明确，明暗层次连贯',
  '画内说者口型同步，画外/OS/VO/系统音依台词标记呈现，听者保持自己的反应；台词清楚，动作音与环境声衬托发声',
  '无对白、OS、旁白字幕条，无BGM、水印及额外logo；必要屏幕信息保留'
 ].join('。')+'。';
}
