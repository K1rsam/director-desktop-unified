import assert from 'node:assert/strict';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { installSyntheticExecutionPolicy } from './synthetic-execution-policy.mjs';

let networkRequests = 0;
globalThis.location = { search: '?preview=1' };
globalThis.fetch = async () => { networkRequests++; throw Error('Network is forbidden'); };
const workspace = { project: { id: 'offline', title: '范围测试' }, source: { text: '离线剧本', version: 2, sha256: 'source', episodes: Array.from({ length: 10 }, (_, i) => ({ id: `ep${i + 1}`, title: `第${i + 1}集`, number: i + 1 })) }, foundation: { version: 1, sourceVersion: 2, sourceSha256: 'source', episodes: [{ episodeId: 'ep7' }] }, assets: [], units: [], media: [], jobs: [{ id: 'finished', kind: 'workflow', entityId: 'foundation.run', provider: 'local', status: 'completed' }], settings: {}, rules: [], providers: [] };
const requests = [];
const props = { workspace, writable: true, mutate: async (...args) => { requests.push(args); return null; }, inspect() {}, onNotice() {}, onError(message) { throw Error(message); }, onTasks() {} };
function mockHookImport(code) {
  const imports = [...code.matchAll(/import\s*\{\s*([^}]+)\s*\}\s*from\s*(['"])react\2\s*;?/g)];
  assert.equal(imports.length, 1, 'Stage1 hook import must be replaced exactly once');
  const names = imports[0][1].split(',').map(name => name.trim()).filter(Boolean);
  assert.ok(names.includes('useState'), 'Stage1 state hook must be mocked');
  assert.equal(new Set(names).size, names.length, 'Duplicate React hook import');
  for (const name of names) assert.ok(['useState', 'useEffect', 'useRef'].includes(name), 'Unmocked React hook: ' + name);
  const replacement = names.map(name => 'const ' + name + ' = (...args) => globalThis.__scopeHooks.' + name + '(...args);').join('\n');
  const result = code.replace(imports[0][0], replacement);
  assert.notEqual(result, code, 'Stage1 hook transformation did not apply');
  return result;
}
assert.throws(() => mockHookImport("import React from 'react';"), /replaced exactly once/);
assert.throws(() => mockHookImport("import { useState, useMemo } from 'react';"), /Unmocked React hook/);
assert.ok(mockHookImport("import {\n useRef, useState, useEffect\n} from \"react\";").includes('globalThis.__scopeHooks.useRef'));
let hookTransforms = 0;
const server = await createServer({ logLevel: 'error', optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, hmr: false }, appType: 'custom', plugins: [{ name: 'scope-interaction-hooks', enforce: 'pre', transform(code, id) {
  if (id.replaceAll('\\', '/').split('?')[0].endsWith('/src/Stage1Panel.tsx')) { hookTransforms++; return mockHookImport(code); }
} }] });
function nodes(tree) { return !tree || typeof tree !== 'object' ? [] : [tree, ...React.Children.toArray(tree.props?.children).flatMap(nodes)]; }
function text(tree) { return typeof tree === 'string' || typeof tree === 'number' ? String(tree) : React.Children.toArray(tree?.props?.children).map(text).join(''); }
let state = [], cursor = 0;
globalThis.__scopeHooks = {
  useState(initial) { const i = cursor++; if (!(i in state)) state[i] = typeof initial === 'function' ? initial() : initial; return [state[i], next => { state[i] = typeof next === 'function' ? next(state[i]) : next; }]; },
  useRef(initial) { const i = cursor++; if (!(i in state)) state[i] = { current: initial }; return state[i]; },
  useEffect() { cursor++; },
};
const stableRef = globalThis.__scopeHooks.useRef(false);
stableRef.current = true; cursor = 0;
assert.strictEqual(globalThis.__scopeHooks.useRef(false), stableRef);
assert.equal(stableRef.current, true); state = []; cursor = 0;
try {
  const { Stage1Panel } = await server.ssrLoadModule('/src/Stage1Panel.tsx');
  assert.equal(hookTransforms, 1, 'Actual Stage1 module did not receive the mock hooks');
  const policy = await installSyntheticExecutionPolicy(server, ['foundation.run']);
  const render = () => { cursor = 0; return Stage1Panel(props); };
  let tree = render();
  nodes(tree).find(node => node.type === 'button' && text(node).startsWith('第7集')).props.onClick();
  tree = render();
  const buttons = nodes(tree).filter(node => node.type?.name === 'Button');
  await buttons.find(node => text(node) === '整理全剧制作基础').props.onClick();
  assert.deepEqual(requests[0][1].options.selection.episodeIds, workspace.source.episodes.map(episode => episode.id), 'Reading episode 7 must not limit the main action');
  await buttons.find(node => text(node) === '仅整理当前集').props.onClick();
  assert.deepEqual(requests[1][1].options.selection.episodeIds, ['ep7'], 'Single episode is a separate explicit action');
  const stage1 = renderToStaticMarkup(tree);
  try {
    policy.offline();
    const offline = renderToStaticMarkup(tree);
    for (const label of ['整理全剧制作基础', '仅整理当前集']) {
      const action = [...offline.matchAll(/<button\b[^>]*>[\s\S]*?<\/button>/g)]
        .find(item => item[0].replace(/<[^>]+>/g, '') === label)?.[0];
      assert.ok(action?.includes('disabled=""'), `Offline policy must disable ${label}`);
    }
  } finally { policy.standard(); }
  assert.ok(stage1.includes('已整理 1 / 10 集')); assert.ok(stage1.includes('尚有 9 集未整理'));
  assert.ok(stage1.includes('部分已整理')); assert.ok(!stage1.includes('已形成基础'));
  const { WorkflowProgress } = await server.ssrLoadModule('/src/WorkflowProgress.tsx');
  const progress = renderToStaticMarkup(React.createElement(WorkflowProgress, { ...props, stage: 'foundation' }));
  assert.ok(progress.includes('本次范围完成 · 全剧未完成')); assert.ok(!progress.includes('制作完成'));
  const complete = { ...workspace, foundation: { ...workspace.foundation, episodes: workspace.source.episodes.map(episode => ({ episodeId: episode.id })) } };
  const completedProgress = renderToStaticMarkup(React.createElement(WorkflowProgress, { ...props, workspace: complete, stage: 'foundation' }));
  assert.ok(completedProgress.includes('已整理 10 / 10 集')); assert.ok(completedProgress.includes('全剧已整理'));
  const { AssetPanel } = await server.ssrLoadModule('/src/AssetPanel.tsx');
  const assets = renderToStaticMarkup(React.createElement(AssetPanel, props));
  assert.ok(assets.includes('当前目录仅覆盖已整理的 第7集'));
  assert.ok(assets.includes('源范围仍待补齐；已交付的分镜资产可独立继续配图。'));
  assert.ok(!assets.includes('请到第一阶段整理全剧制作基础'));
  assert.equal(networkRequests, 0); assert.equal(requests.length, 2);
  console.log(JSON.stringify({ passed: 7, hookTransforms, hookMockGuardsVerified: true, refIdentityVerified: true, method: 'Actual Stage1 React interactions and Stage1/2/progress server rendering with actual policy guard', networkRequests, mutations: '2 isolated mock calls, no backend or media access' }, null, 2));
} finally { await server.close(); delete globalThis.__scopeHooks; }
