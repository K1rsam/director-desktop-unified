import test from 'node:test';
import assert from 'node:assert/strict';
import { managedBrowserReady, managedBrowserStatus } from './managedBrowser.ts';

const ready = () => ({ schemaVersion: 'managed-browser/v1', ok: true, state: 'ready', reason: 'ready', message: '已连接', installed: true, running: true, connected: true, authenticated: true, ready: true, extensionReady: true, process: { owned: true, running: true, pid: 12 }, profileId: '11111111-1111-4111-8111-111111111111', newBinding: false, modelSubmissions: 0, videoSubmissions: 0 });

test('专属浏览器只有全部证据齐全才允许绑定', () => {
  assert.equal(managedBrowserReady(managedBrowserStatus(ready())), true);
  for (const key of ['installed', 'running', 'connected', 'authenticated', 'ready', 'extensionReady', 'ok']) {
    assert.equal(managedBrowserReady(managedBrowserStatus({ ...ready(), [key]: false })), false, key);
  }
  for (const patch of [{ requiresLogin: true }, { state: 'login_required' }, { profileId: undefined }, { process: { owned: false, running: true } }]) {
    assert.equal(managedBrowserReady(managedBrowserStatus({ ...ready(), ...patch })), false);
  }
});

test('未启动状态可以显示，缺字段及意外生成回执必须拒绝', () => {
  assert.equal(managedBrowserReady(managedBrowserStatus({ ...ready(), ok: false, state: 'not_started', running: false, connected: false, authenticated: false, ready: false, extensionReady: false, profileId: undefined, process: { owned: false, running: false } })), false);
  for (const patch of [{ reason: undefined }, { process: undefined }, { process: { owned: true, running: true, pid: -1 } }, { newBinding: true }, { modelSubmissions: 1 }, { videoSubmissions: 1 }, { profileId: 'unverified' }]) {
    assert.throws(() => managedBrowserStatus({ ...ready(), ...patch }));
  }
});
