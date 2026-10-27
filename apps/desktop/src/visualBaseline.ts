import type { Settings, VisualBaseline } from './types';

export const coverageLabels = { form: '画风与造型', rendering: '渲染与着色', materials: '材质', colorLighting: '色彩与光影', aspectRatio: '画幅', localExceptions: '局部风格例外' } as const;
export type CoverageKey = keyof typeof coverageLabels;
export const visualPresets = {
  '自然3D国漫': {
    form: '自然三维国漫造型，人物比例、骨相、发束与服装结构连贯，场景体块和动作接触清楚',
    rendering: '柔和连续的三维明暗塑形，风格化CG表面细节，不使用赛璐璐硬切色块或实拍毛孔观感',
    materials: '布料纤维、皮革、金属与石材有不同粗糙度和反射响应，肤质柔和且不过度镜面泛亮',
    colorLighting: '统一色彩与冷暖关系，主光及反射光构成自然层次，暗部可读，避免过曝和无依据的浓重滤镜',
  },
  '3D三渲二国漫': {
    form: '风格化三维建模与国漫比例，人物脸型、发束、服装轮廓及空间体块清楚稳定',
    rendering: '三渲二着色，赛璐璐分层明暗主导人物和环境观感，避免摄影级皮肤与写实CG偏移',
    materials: 'PBR用于区分布料、皮革、金属和石材的粗糙度与反射；皮肤高光克制，避免统一塑料泛亮',
    colorLighting: '色彩关系统一、饱和度适中，主光方向清楚，亮面、暗面及接触阴影形成可读层次',
  },
  '2D动漫': {
    form: '清晰稳定的二维角色线稿，脸型、造型比例与线条粗细连贯，背景空间关系可读',
    rendering: '分层平涂色块与二维动画明暗，不将现实部分改成三维写实渲染',
    materials: '以纹理笔触、色块与高光形状区分衣料、金属、石材等表面，不强加写实微表面',
    colorLighting: '统一色板与明暗层级，人物从背景清楚分离，前中后景有层次而不过度虚化',
  },
  '真人电影写实': {
    form: '电影写实人物与自然比例，脸型、年龄、发型和服装结构稳定，人物与实景尺度一致',
    rendering: '电影摄影的连续明暗和真实光学质感，不使用动漫描边、赛璐璐色块或塑料CG皮肤',
    materials: '肤质、衣料、皮革、金属与石材保留各自纹理和真实反射，接触处有可信阴影',
    colorLighting: '统一电影色彩关系，光线方向明确，以主辅光、反射和适度景深组织空间，保留肤色与暗部细节',
  },
} as const;
export type VisualPreset = keyof typeof visualPresets;
const exceptions = '局部二维平涂、图解或Q版仅按已采用镜头的主体、起止与回接执行；三维Q版不自动转二维，其余现实画面保持项目主画风';
const blankCoverage = (): VisualBaseline['coverage'] => ({ form: '', rendering: '', materials: '', colorLighting: '', aspectRatio: '', localExceptions: '' });
const validRatio = (value: unknown): value is string => typeof value === 'string' && /^[1-9]\d*:[1-9]\d*$/.test(value);
export function currentVisualText(settings: Settings): string { return settings.projectVisualStyle || settings.visualStyle || ''; }

// Building a candidate never claims adoption or changes sound/fantasy/channel settings.
export function visualCandidate(settings: Settings, text: string, coverage = settings.visualBaseline?.coverage ?? blankCoverage()): VisualBaseline {
  const old = settings.visualBaseline;
  return { version: 'series-visual-baseline-v1', id: old?.id || `visual-${crypto.randomUUID()}`, revision: old ? old.revision + (old.status === 'adopted' ? 1 : 0) : 1, status: 'candidate', visualStyle: text, coverage: { ...coverage } };
}
export function visualBaselinePatch(candidate: VisualBaseline): Partial<Settings> {
  return { visualBaseline: candidate, projectVisualStyle: candidate.visualStyle, visualStyle: candidate.visualStyle };
}
export function proposeVisualPreset(settings: Settings, name: VisualPreset): Partial<Settings> {
  if (!Object.hasOwn(visualPresets, name) || !validRatio(settings.ratio)) throw new Error('请先选择画面比例，再选择画风预设。');
  const coverage = { ...visualPresets[name], aspectRatio: `画幅${settings.ratio}`, localExceptions: exceptions };
  const text = [`项目视觉基准：${name}`, coverage.aspectRatio, coverage.form, coverage.rendering, coverage.materials, coverage.colorLighting, coverage.localExceptions].join('。') + '。';
  return visualBaselinePatch(visualCandidate(settings, text, coverage));
}
export function baselineProblems(settings: Settings): string[] {
  const b = settings.visualBaseline;
  if (!b) return ['请选择明确画风预设，或为现有自定义说明补齐六项视觉依据。'];
  const problems: string[] = [];
  if (b.version !== 'series-visual-baseline-v1' || !b.id?.trim() || /[\r\n\0]/.test(b.id) || !Number.isSafeInteger(b.revision) || b.revision <= 0 || !['candidate', 'adopted'].includes(b.status)) problems.push('视觉基准身份或版本无效，请重新建立基准。');
  if (!b.visualStyle.trim() || /[\r\0]/.test(b.visualStyle)) problems.push('请填写完整视觉说明。');
  for (const key of Object.keys(coverageLabels) as CoverageKey[]) {
    const excerpt = b.coverage?.[key];
    if (!excerpt?.trim() || /[\r\0]/.test(excerpt) || !b.visualStyle.includes(excerpt)) problems.push(`${coverageLabels[key]}：请填写视觉说明中对应的原文片段。`);
  }
  if (new Set(Object.values(b.coverage ?? {})).size <= 1) problems.push('六项依据不能全部使用同一个宽泛画风标签。');
  if (!validRatio(settings.ratio) || !b.coverage?.aspectRatio.match(/\d+:\d+/g)?.includes(settings.ratio)) problems.push('画幅依据须包含当前画面比例；修改比例后请同步完整视觉说明。');
  if (settings.visualStyle !== b.visualStyle || settings.projectVisualStyle !== b.visualStyle) problems.push('项目画风与完整视觉说明不同，请重新确认。');
  return problems;
}
export function baselineAdopted(settings: Settings): boolean { return settings.visualBaseline?.status === 'adopted' && !!settings.visualBaseline.adoptionBasis?.trim() && !baselineProblems(settings).length; }
export function adoptVisualBaseline(settings: Settings): Partial<Settings> {
  const problems = baselineProblems(settings);
  if (problems.length || !settings.visualBaseline) throw new Error(problems.join(' '));
  return visualBaselinePatch({ ...settings.visualBaseline, status: 'adopted', adoptionBasis: `用户在项目制作设置明确确认采用此完整视觉基准，画面比例 ${settings.ratio}。` });
}
export function invalidateVisualBaselineForRatio(settings: Settings, patch: Partial<Settings>): Partial<Settings> {
  if (!Object.hasOwn(patch, 'ratio') || patch.ratio === settings.ratio || !settings.visualBaseline) return patch;
  return { ...patch, visualBaseline: visualCandidate(settings, settings.visualBaseline.visualStyle) };
}
