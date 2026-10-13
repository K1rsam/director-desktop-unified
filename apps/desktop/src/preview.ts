import type { Workspace } from './types';

const illustration = (label: string, color: string) => 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="640" height="480" viewBox="0 0 640 480"><defs><linearGradient id="g" x2="1" y2="1"><stop stop-color="#e1e6f6"/><stop offset="1" stop-color="${color}"/></linearGradient></defs><rect width="640" height="480" fill="url(#g)"/><circle cx="320" cy="172" r="58" fill="#a49abb" opacity=".45"/><path d="M215 405v-90q0-78 105-78t105 78v90" fill="#a49abb" opacity=".4"/><text x="320" y="445" text-anchor="middle" font-size="19" fill="#67597f" font-family="sans-serif">${label} · 预览示意图</text></svg>`);

export const previewWorkspace: Workspace = {
  project: { id: 'preview-project', title: '归途 · 界面预览', updatedAt: '2026-10-04', counts: { assets: 3, units: 2 } },
  source: { version: 1, text: '第一集\n场景：老屋门前，傍晚\n林夏推开院门，父亲坐在台阶上。\n林夏：爸，我回来了。\n父亲抬头，握着钥匙的手停在半空。', sha256: '预览资料', episodes: [{ id: 'ep-1', number: 1, title: '归途' }] },
  foundation: { direction: '16:9 横屏真人。以克制表演与自然晚光呈现久别重逢。', status: 'ready', notes: '此处是明确标注的界面示例，未执行生成。' },
  assets: [
    { id: 'summer', variantId: 'travel', name: '林夏 · 返乡装', kind: 'character', version: 1, prompt: '26岁中国女性，齐肩黑发，自然五官，浅色衬衫、深灰长裤，肩背旧帆布包。全身中性站姿，均匀柔光，16:9 真人参考图，保留清晰服装与面部特征。', adoptedMediaId: 'summer-image', applicability: { episodeIds: ['ep-1'] }, status: 'adopted' },
    { id: 'father', variantId: 'home', name: '父亲 · 居家装', kind: 'character', version: 1, prompt: '62岁中国男性，短灰发，旧深蓝布衫，安静坐姿，真实皮肤纹理。', adoptedMediaId: 'father-image', applicability: { episodeIds: ['ep-1'] }, status: 'adopted' },
    { id: 'house', variantId: 'dusk', name: '老屋门前 · 傍晚', kind: 'scene', version: 1, prompt: '乡村老屋院门，旧青砖与木门，自然晚光，16:9 横屏真人场景。', status: 'needs_image' },
  ],
  media: [
    { id: 'summer-image', kind: 'image', path: '', previewUrl: illustration('林夏', '#ded5e9'), assetId: 'summer', variantId: 'travel', adopted: true },
    { id: 'father-image', kind: 'image', path: '', previewUrl: illustration('父亲', '#d3dbe9'), assetId: 'father', variantId: 'home', adopted: true },
  ],
  units: [
    { id: 'U01', title: '推开院门', episodeId: 'ep-1', episodeNumber: 1, scene: '老屋门前', duration: 12, version: 1, status: 'needs_media', prompt: `16:9 横屏真人，12秒。傍晚，老屋院门外。
林夏肩背旧帆布包，手指扶住门沿，停顿半拍后推开木门。镜头从门外中景缓慢推进到她的侧脸，保持空间方向明确。院内父亲坐在台阶上。
林夏轻声说：“爸，我回来了。”声音从第5秒开始，到第7秒结束；随后保留两秒呼吸与眼神变化。父亲抬头，握着钥匙的手停在半空。
自然晚光从门侧进入，肤色自然，环境音保留轻微风声和门轴声。`, assetRefs: [{ assetId: 'summer', variantId: 'travel', role: 'character', order: 1 }, { assetId: 'father', variantId: 'home', role: 'character', order: 2 }, { assetId: 'house', variantId: 'dusk', role: 'scene', order: 3 }], problems: ['老屋门前 · 傍晚尚未采用参考图'] },
    { id: 'U02', title: '父亲的停顿', episodeId: 'ep-1', episodeNumber: 1, duration: 10, version: 1, status: 'draft', prompt: `16:9 横屏真人，10秒。老屋门前，傍晚。
父亲坐在台阶上，身穿旧深蓝布衫。听见林夏声音后，手中钥匙停住，缓缓抬头。近景保持三秒静默，视线朝向院门。微弱风声与钥匙轻碰声，克制表演。`, assetRefs: [{ assetId: 'father', variantId: 'home', role: 'character', order: 1 }] },
  ],
  jobs: [], settings: { textProvider: 'codex', textModel: 'gpt-6.1', reasoning: 'medium', imageProvider: 'dreamina', imageModel: '5.0', imageResolution: '2k', videoProvider: 'dreamina', videoModel: 'seedance2.5', videoResolution: '720p', ratio: '16:9' },
  dreaminaAccount: { status: 'unknown', ready: false, identityVerified: false, message: '界面预览未读取真实登录状态或积分；请在桌面客户端连接即梦。' },
  providers: [{ id: 'dreamina', name: '即梦官方 CLI', available: false, message: '示例模型目录，预览模式未检查实际连接', models: [{ id: '5.0', kind: 'image', adapted: true, ratios: ['21:9', '16:9', '3:2', '4:3', '1:1', '3:4', '2:3', '9:16'], resolutions: ['2k', '4k', '1080p'] }, { id: '5.0Pro', kind: 'image', adapted: true, ratios: ['21:9', '16:9', '3:2', '4:3', '1:1', '3:4', '2:3', '9:16'], resolutions: ['1.5k', '2k', '4k', '1080p'] }, { id: 'seedance2.5', kind: 'video', adapted: true, ratios: ['1:1', '3:4', '16:9', '4:3', '9:16', '21:9'], resolutions: ['480p', '720p', '1080p'], minDuration: 4, maxDuration: 30, images: 30 }] }, { id: 'bh', name: 'BH API', available: false, message: '预览模式未配置' }, { id: 'chrome', name: 'ChatGPT 网页', available: false, message: '预览模式未配置' }],
  rules: [{ id: 'screenplay-fidelity', version: '1.0.0', stage: 1 }, { id: 'story-foundation', version: '1.0.0', stage: 1 }, { id: 'asset-art-director', version: '1.0.0', stage: 2 }, { id: 'asset-image-production', version: '1.0.0', stage: 2 }, { id: 'episode-emotion-planner', version: '1.0.0', stage: 3 }, { id: 'unit-performance-director', version: '1.0.0', stage: 3 }, { id: 'episode-finishing-editor', version: '1.0.0', stage: 3 }, { id: 'unit-video-production', version: '1.0.0', stage: 4 }],
};

// Read-only design evidence, never loaded as a production source or a model result.
const previewText = previewWorkspace.source!.text!;
let previewOffset = 0;
previewWorkspace.source!.scenes = [{ id: 'sc-1', episodeId: 'ep-1', title: '老屋门前，傍晚' }];
previewWorkspace.source!.rows = previewText.split(/(?<=\n)/).map((text, index) => {
  const range = { start: previewOffset, end: previewOffset + text.length }; previewOffset += text.length;
  return { id: `row-${index + 1}`, number: index + 1, text, range, kind: index === 0 ? 'episode_header' : index === 1 ? 'scene_header' : 'body', episodeId: 'ep-1', sceneId: index > 0 ? 'sc-1' : undefined };
});
previewWorkspace.foundation = { ...previewWorkspace.foundation, version: 1, sourceVersion: 1, productionDirection: previewWorkspace.foundation!.direction, directionConfirmed: true, overview: '林夏返乡，父亲的动作停顿承接两人的久别重逢。以下仅为界面示例。', episodes: [{ episodeId: 'ep-1', summary: '院门开启，林夏说出回家后的第一句话。', context: '旧屋、钥匙与短暂停顿形成必要视觉证据。' }], facts: [{ id: 'preview-fact', assetId: 'summer', variantId: 'travel', attribute: '动作', value: '林夏推开院门', scope: 'STATE', sourceRefs: ['row-3'] }], issues: [] };
previewWorkspace.units = previewWorkspace.units.map((unit, index) => ({ ...unit, sceneId: 'sc-1', sourceRefs: index === 0 ? ['row-3', 'row-4'] : ['row-5'], initialState: index === 0 ? '林夏站在院门外，父亲坐在台阶上。' : '父亲听见林夏的声音，钥匙仍握在手中。', exitState: index === 0 ? '林夏停在门内，父亲抬头。' : '父亲抬头望向院门，手停在半空。', shots: [{ id: `shot-${index + 1}`, duration: unit.duration, picture: index === 0 ? '林夏扶住门沿，推门进入，轻声开口。' : '父亲握钥匙的手停住，缓缓抬头。', camera: index === 0 ? '门外中景缓慢推进侧脸' : '近景保持三秒停顿', lighting: '自然晚光从院门侧进入', sound: '轻微风声与门轴声', dialogueParts: index === 0 ? [{ lineId: 'line-4', text: '爸，我回来了。', speaker: '林夏', channel: '现场', presentation: '画内', delivery: '轻声', voiceStart: 5, voiceEnd: 7 }] : [], transition: '切到父亲的停顿' }] }));
previewWorkspace.rules = previewWorkspace.rules.map(rule => ({ ...rule, version: rule.stage === 1 ? '1.1.0' : rule.stage === 3 ? '2.0.0' : rule.version }));
