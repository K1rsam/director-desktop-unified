import test from 'node:test';
import assert from 'node:assert/strict';
import { foundationCoverage, foundationRunOptions } from './foundationCoverage.ts';
import type { Workspace } from './types.ts';

function fixture(): Workspace {
  return { project: { id: 'p', title: '覆盖测试' }, source: { version: 2, sha256: 'current-source', episodes: Array.from({ length: 10 }, (_, i) => ({ id: `ep${i + 1}`, number: i + 1, title: `第${i + 1}集` })) },
    foundation: { version: 3, sourceVersion: 2, sourceSha256: 'current-source', episodes: [{ episodeId: 'ep7', summary: '本集资料' }] },
    assets: [], units: [], media: [], jobs: [{ id: 'run', provider: 'local', status: 'completed', kind: 'workflow', entityId: 'foundation.run' }], providers: [], rules: [], settings: {} };
}

test('单集已完成任务不能将整剧覆盖判定为完成', () => {
  const result = foundationCoverage(fixture());
  assert.equal(result.count, 1); assert.equal(result.total, 10); assert.equal(result.complete, false);
  assert.deepEqual(result.completedIds, ['ep7']); assert.equal(result.missingIds.length, 9);
  assert.equal(result.summary, '已整理 1 / 10 集'); assert.deepEqual(result.coveredTitles, ['第7集']);
});
test('默认提交全剧，单集提交必须明确传入且限定当前剧本', () => {
  const workspace = fixture();
  assert.equal(foundationRunOptions(workspace).selection.episodeIds.length, 10);
  assert.deepEqual(foundationRunOptions(workspace, 'ep7').selection.episodeIds, ['ep7']);
  assert.deepEqual(foundationRunOptions(workspace, 'unknown').selection.episodeIds, []);
});
test('源版本和哈希分别变化都使旧覆盖失效', () => {
  for (const change of [{ version: 3 }, { sha256: 'changed' }, { sha256: undefined }]) {
    const workspace = fixture(); Object.assign(workspace.source!, change);
    const result = foundationCoverage(workspace);
    assert.equal(result.count, 0); assert.equal(result.stale, true); assert.equal(result.complete, false);
  }
});
test('累计各集已发布结果去重，仅认当前剧本范围', () => {
  const workspace = fixture();
  workspace.foundation!.episodes = [...workspace.source!.episodes!.map(episode => ({ episodeId: episode.id })), { episodeId: 'ep7' }, { episodeId: 'old' }, null, {}];
  const result = foundationCoverage(workspace);
  assert.equal(result.count, 10); assert.equal(result.complete, true); assert.deepEqual(result.missingIds, []);
});
test('空剧本、空基础和只有资产均不推断覆盖', () => {
  const workspace = fixture(); workspace.foundation = null;
  workspace.assets = [{ id: 'asset', name: '人工资产', applicability: { episodes: ['ep7'] } }];
  assert.equal(foundationCoverage(workspace).count, 0);
  workspace.source = null;
  assert.equal(foundationCoverage(workspace).total, 0); assert.equal(foundationCoverage(workspace).complete, false);
});
