import type { Settings } from './types';
import { adoptVisualBaseline, baselineAdopted, baselineProblems, coverageLabels, currentVisualText, proposeVisualPreset, visualBaselinePatch, visualCandidate, visualPresets } from './visualBaseline';
import type { CoverageKey, VisualPreset } from './visualBaseline';

type FantasyMode = 'recommend' | 'off' | 'all' | 'review';
type Props = { settings: Settings; writable: boolean; onChange: (patch: Partial<Settings>) => void };

export function displayedFantasyMode(settings: Settings): FantasyMode {
  const mode = settings.fantasyMode;
  if (mode === 'off' || mode === 'all') return mode;
  return mode === 'review' && settings.requireFantasyReview === true ? 'review' : 'recommend';
}
export function fantasySettingsPatch(mode: FantasyMode): Partial<Settings> {
  return { fantasyMode: mode, requireFantasyReview: mode === 'review' };
}
export function ProductionSettings({ settings, writable, onChange }: Props) {
  const problems = baselineProblems(settings), adopted = baselineAdopted(settings);
  return <><label>幻想表达<select disabled={!writable} value={displayedFantasyMode(settings)} onChange={event => onChange(fantasySettingsPatch(event.target.value as FantasyMode))}>
    <option value="recommend">自动采用推荐</option><option value="off">不增加额外幻想</option><option value="all">采用全部候选</option><option value="review">先审阅候选</option>
  </select><small className="control-hint">默认自动采用推荐候选并继续制作。选择先审阅后，会等你确认候选。</small></label>
  <label>项目画风<select aria-label="选择视觉基准预设" disabled={!writable} value="" onChange={event => {
    const name = event.target.value;
    if (name === 'custom') onChange(visualBaselinePatch(visualCandidate(settings, currentVisualText(settings))));
    else if (name) onChange(proposeVisualPreset(settings, name as VisualPreset));
  }}><option value="">选择预设或保留现有说明</option>{Object.keys(visualPresets).map(name => <option key={name} value={name} disabled={!/^[1-9]\d*:[1-9]\d*$/.test(String(settings.ratio || ''))}>{name}</option>)}<option value="custom">完整自定义</option></select>
  <small className="control-hint">先选择画面比例。应用预设会替换下方说明，采用前可修改；保留原画风时可直接补齐依据。</small></label>
  <label>完整视觉说明<textarea aria-label="完整视觉说明" disabled={!writable} rows={6} value={currentVisualText(settings)} onChange={event => onChange(visualBaselinePatch(visualCandidate(settings, event.target.value)))} placeholder="明确造型、渲染、材质、光影、画幅及局部风格例外"/></label>
  <details><summary>查看与编辑六项视觉依据</summary><small className="control-hint">每项填写上方完整说明中的原文片段。预设已填好；自定义时需补齐。</small>{(Object.keys(coverageLabels) as CoverageKey[]).map(key => <label key={key}>{coverageLabels[key]}<textarea aria-label={coverageLabels[key]} disabled={!writable} value={settings.visualBaseline?.coverage?.[key] || ''} onChange={event => {
    const candidate = visualCandidate(settings, currentVisualText(settings));
    candidate.coverage[key] = event.target.value;
    onChange(visualBaselinePatch(candidate));
  }}/></label>)}</details>
  <div><p role="status">{adopted ? '视觉基准已确认采用' : settings.visualBaseline ? '视觉基准待确认' : '保留旧画风，尚未补齐完整视觉基准'}</p>
    {!adopted && problems.length > 0 && <ul>{problems.map(problem => <li key={problem}>{problem}</li>)}</ul>}
    <button type="button" disabled={!writable || adopted || problems.length > 0} onClick={() => onChange(adoptVisualBaseline(settings))}>采用此视觉基准</button>
    <small className="control-hint">确认后请保存项目设置。修改说明或画幅需重新采用，不重写旧稿、旧图片或历史请求。</small>
  </div>
  <label>制作美术方向<textarea disabled={!writable} value={String(settings.productionDirection || '')} onChange={event => onChange({ productionDirection: event.target.value, productionDirectionConfirmed: false })} placeholder="补充服装、材质与审美方向，不覆盖原文年龄、肤色、体型或时代"/></label>
  <label><input type="checkbox" disabled={!writable || !String(settings.productionDirection || '').trim()} checked={settings.productionDirectionConfirmed === true} onChange={event => onChange({ productionDirectionConfirmed: event.target.checked })}/>确认采用此制作美术方向</label>
  <label>人物图片展示<textarea disabled={!writable} value={typeof settings.imagePresentation === 'string' ? settings.imagePresentation : settings.imagePresentation?.composition || ''} onChange={event => onChange({ imagePresentation: event.target.value })} placeholder="默认：左侧大幅肖像，右侧正／侧／背全身，完整头脚，干净中性背景，无资料栏"/><small className="control-hint">仅用于人物基础与派生形象；资产已有明确构图优先，场景、道具、群体和非人角色不套人物四视图。</small></label></>;
}
