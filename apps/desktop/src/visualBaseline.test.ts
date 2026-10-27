import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import * as React from 'react';
import { createServer } from 'vite';
import { adoptVisualBaseline, baselineAdopted, baselineProblems, currentVisualText, invalidateVisualBaselineForRatio, proposeVisualPreset, visualBaselinePatch, visualCandidate, visualPresets } from './visualBaseline.ts';
import type { Settings } from './types.ts';

const original: Settings = { ratio: '16:9', projectVisualStyle: '原来 3D 画风，不改字。', visualStyle: '原来 3D 画风，不改字。', fantasyMode: 'off', requireFantasyReview: false, prohibitions: '无背景音乐，无字幕', chromeToken: 'synthetic-only' };
function candidate(): Settings { return { ...original, ...proposeVisualPreset(original, '自然3D国漫') }; }
function adopted(): Settings { const settings = candidate(); return { ...settings, ...adoptVisualBaseline(settings) }; }

test('all explicit presets create complete candidates without changing other policies', () => {
  for (const name of Object.keys(visualPresets) as (keyof typeof visualPresets)[]) {
    const patch = proposeVisualPreset(original, name), settings = { ...original, ...patch };
    assert.deepEqual(Object.keys(patch).sort(), ['projectVisualStyle', 'visualBaseline', 'visualStyle']);
    assert.deepEqual(baselineProblems(settings), []);
    assert.equal(settings.visualBaseline!.status, 'candidate');
    assert.equal(baselineAdopted(settings), false);
    assert.equal(settings.visualBaseline!.adoptionBasis, undefined);
    for (const key of ['fantasyMode', 'requireFantasyReview', 'prohibitions', 'chromeToken']) assert.equal(settings[key], original[key]);
    assert.doesNotMatch(settings.visualStyle!, /背景音乐|字幕|台词|配音/);
  }
  assert.equal(currentVisualText(original), '原来 3D 画风，不改字。');
});

test('custom text is preserved exactly and missing or paraphrased coverage is rejected', () => {
  const custom = visualCandidate(original, currentVisualText(original));
  const settings = { ...original, ...visualBaselinePatch(custom) };
  assert.equal(custom.visualStyle, original.projectVisualStyle);
  assert.ok(baselineProblems(settings).length >= 6);
  assert.throws(() => adoptVisualBaseline(settings));
  const complete = candidate();
  complete.visualBaseline!.coverage.materials = '没有出现在正文中的材质摘要';
  assert.ok(baselineProblems(complete).some(message => message.startsWith('材质')));
});

test('adoption is explicit; editing text or ratio revokes adoption and preserves identity', () => {
  const first = adopted();
  assert.equal(baselineAdopted(first), true);
  assert.match(first.visualBaseline!.adoptionBasis!, /用户.*明确确认/);
  const text = currentVisualText(first) + '自定义补充。';
  const changed = visualCandidate(first, text);
  assert.equal(changed.id, first.visualBaseline!.id);
  assert.equal(changed.revision, 2);
  assert.equal(changed.status, 'candidate');
  assert.equal(changed.adoptionBasis, undefined);
  assert.equal(visualCandidate({ ...first, ...visualBaselinePatch(changed) }, text + '再次编辑').revision, 2);
  const ratioChange = invalidateVisualBaselineForRatio(first, { ratio: '9:16' });
  const next = { ...first, ...ratioChange };
  assert.equal(next.visualBaseline!.status, 'candidate');
  assert.equal(next.visualStyle, first.visualStyle);
  assert.ok(baselineProblems(next).some(message => message.includes('画幅依据')));
  assert.deepEqual(invalidateVisualBaselineForRatio(first, { ratio: '16:9' }), { ratio: '16:9' });
  assert.deepEqual(invalidateVisualBaselineForRatio(original, { ratio: '9:16' }), { ratio: '9:16' });
});

test('invalid baseline identity and substring ratios cannot be adopted', () => {
  assert.throws(() => proposeVisualPreset({ ratio: '' }, '自然3D国漫'));
  const settings = candidate();
  settings.visualBaseline!.revision = 0;
  assert.throws(() => adoptVisualBaseline(settings));
  const otherRatio = { ...original, ...proposeVisualPreset({ ratio: '116:9' }, '自然3D国漫') };
  assert.ok(baselineProblems(otherRatio).some(message => message.includes('画幅依据')));
});

function nodes(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}
test('project UI requires an adoption click, persists baseline, and invalidates it on ratio change', async () => {
  Object.assign(globalThis, { location: { search: '' }, window: {} });
  const server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom' });
  try {
    const { ProductionSettings } = await server.ssrLoadModule('/src/ProductionSettings.tsx');
    const { ProjectSettingsPanel } = await server.ssrLoadModule('/src/Panels.tsx');
    const { previewWorkspace } = await server.ssrLoadModule('/src/preview.ts');
    let current = { ...original }, changes = 0;
    const renderProduction = () => nodes(ProductionSettings({ settings: current, writable: true, onChange: (patch: Settings) => { changes++; current = { ...current, ...patch }; } }));
    let view = renderProduction();
    assert.equal(changes, 0, 'viewing legacy settings cannot adopt or alter them');
    assert.equal(view.find(n => n.props?.['aria-label'] === '完整视觉说明').props.value, original.visualStyle);
    view.find(n => n.props?.['aria-label'] === '选择视觉基准预设').props.onChange({ target: { value: '自然3D国漫' } });
    assert.equal(current.visualBaseline!.status, 'candidate');
    view = renderProduction();
    const adopt = view.find(n => n.type === 'button' && n.props.children === '采用此视觉基准');
    assert.equal(adopt.props.type, 'button'); assert.equal(adopt.props.disabled, false);
    adopt.props.onClick(); assert.equal(baselineAdopted(current), true);

    let state: Settings = current;
    let providers = previewWorkspace.providers;
    const saves: any[] = [];
    const internals = (React as any).__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;
    const renderProject = () => {
      const previous = internals.H;
      internals.H = { useState() { return [state, (update: any) => { state = typeof update === 'function' ? update(state) : update; }]; } };
      try { return nodes(ProjectSettingsPanel({ workspace: { ...previewWorkspace, providers, settings: current }, writable: true, mutate: async (action: string, payload: any) => { saves.push({ action, payload }); return null; } })); }
      finally { internals.H = previous; }
    };
    view = renderProject();
    view.find(n => n.type === 'form').props.onSubmit({ preventDefault() {} });
    assert.equal(saves.length, 1);
    assert.deepEqual(saves[0].payload.settings.visualBaseline, current.visualBaseline);
    assert.equal(saves[0].payload.settings.chromeToken, undefined, 'project settings do not overwrite global credentials');
    const ratioLabel = view.find(n => n.type === 'label' && Array.isArray(n.props.children) && n.props.children[0] === '画面比例');
    nodes(ratioLabel).find(n => n.type === 'select').props.onChange({ target: { value: '9:16' } });
    assert.equal(state.visualBaseline!.status, 'candidate');
    assert.equal(state.visualBaseline!.revision, 2);
    view = renderProject();
    view.find(n => n.type === 'form').props.onSubmit({ preventDefault() {} });
    assert.equal(saves[1].payload.settings.visualBaseline.status, 'candidate', 'draft settings remain saveable');

    // Text-only projects need no media account/model to choose and save a baseline.
    state = {}; providers = [];
    view = renderProject();
    const emptyRatioLabel = view.find(n => n.type === 'label' && Array.isArray(n.props.children) && n.props.children[0] === '画面比例');
    const emptyRatioSelect = nodes(emptyRatioLabel).find(n => n.type === 'select');
    assert.equal(emptyRatioSelect.props.value, '', 'no implicit displayed 16:9');
    assert.deepEqual(nodes(emptyRatioSelect).filter(n => n.type === 'option').map(n => n.props.value), ['', '16:9', '9:16', '1:1', '4:3', '3:4']);
    assert.equal(state.visualBaseline, undefined);
    emptyRatioSelect.props.onChange({ target: { value: '9:16' } });
    view = renderProject();
    let production = nodes(ProductionSettings(view.find(n => n.type === ProductionSettings).props));
    production.find(n => n.props?.['aria-label'] === '选择视觉基准预设').props.onChange({ target: { value: '2D动漫' } });
    assert.equal(state.visualBaseline!.status, 'candidate');
    view = renderProject();
    production = nodes(ProductionSettings(view.find(n => n.type === ProductionSettings).props));
    const textOnlyAdopt = production.find(n => n.type === 'button' && n.props.children === '采用此视觉基准');
    assert.equal(textOnlyAdopt.props.disabled, false);
    textOnlyAdopt.props.onClick();
    view = renderProject();
    view.find(n => n.type === 'form').props.onSubmit({ preventDefault() {} });
    assert.equal(saves.length, 3);
    assert.equal(saves[2].payload.settings.ratio, '9:16');
    assert.equal(baselineAdopted(saves[2].payload.settings), true);
    assert.equal(saves[2].payload.settings.imageModel, undefined);
    assert.equal(saves[2].payload.settings.videoModel, undefined);
  } finally { await server.close(); }
});
