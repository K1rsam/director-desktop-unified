import test from 'node:test';
import assert from 'node:assert/strict';
import { foundationParallelResume } from './foundationTrial.ts';
import type { Workspace } from './types.ts';

function fixture(): Workspace {
  return { project: { id: 'p', title: '三路续跑' }, source: { version: 1, sha256: 'source', episodes: [{ id: 'e1' }, { id: 'e2' }] },
    foundation: {}, assets: [], units: [], media: [], rules: [], providers: [], settings: {},
    jobs: [{ id: 'run', kind: 'workflow', entityId: 'foundation.run', provider: 'local', status: 'failed', error: 'F2完整请求超过200k应用预算；原文与已接受响应保留，未截断或提交',
      snapshot: { runnerVersion: 'desktop-runner/v1', stage: 'foundation.run', sourceVersion: 1, input: { source: { version: 1, sha256: 'source' }, selection: { episodeIds: ['e1', 'e2'] } } } }],
    workflowProgress: [{ workflowId: 'run', stage: 'foundation.run', status: 'failed', nodes: [{ id: 'a', status: 'accepted', stage: 'foundation:F2', scope: ['e1'], delivery: { method: 'foundation-delivery/v2', fragment: null } }] }],
  };
}

test('容量停止且完整前缀已保存时前台继续绑定原workflow，不新建或猜造批次', () => {
  const workspace=fixture(), before=JSON.stringify(workspace);
  assert.deepEqual(foundationParallelResume(workspace,workspace.jobs[0]), { workflowId:'run', foundationParallelTrial:{version:'foundation-three-lane-trial/v1',concurrency:3} });
  assert.equal(JSON.stringify(workspace),before);
});

test('在途、未知、旧源、分片、已开始核对和普通网络错误不显示三路迁移入口', () => {
  const mutate: ((w:Workspace)=>void)[] = [
    w=>{w.jobs[0].status='running';}, w=>{w.jobs[0].submissionUnknown=true;},
    w=>{w.jobs[0].error='tls handshake eof';}, w=>{w.source!.version=2;},
    w=>{const nodes=w.workflowProgress![0].nodes as Record<string,unknown>[];Object.assign(nodes[0],{delivery:{method:'foundation-delivery/v2',fragment:{index:0,count:2}}});},
    w=>{const nodes=w.workflowProgress![0].nodes as Record<string,unknown>[];nodes.push({id:'q',status:'accepted',stage:'foundation:Q1',scope:['e1']});},
  ];
  for(const change of mutate){const workspace=fixture();change(workspace);assert.equal(foundationParallelResume(workspace,workspace.jobs[0]),null);}
});
