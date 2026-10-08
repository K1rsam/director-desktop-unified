import test from 'node:test';
import assert from 'node:assert/strict';
import { promises as fs } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { packageReadiness, resourceFiles, pinned } from '../scripts/package-readiness.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const protocol = 'director-sdk-bridge/v1';
const request = (method, index) => ({ protocolVersion: protocol, requestId: `health_${index + 1}`, attemptId: null, method, params: null });
async function worker(bundle, temp, requests, extraArgs = []) {
  const audit = path.join(temp, 'audit.json');
  const home = path.join(temp, 'isolated-home');
  await fs.mkdir(home, { recursive: true });
  const child = spawn(process.execPath, ['--import', pathToFileURL(path.join(root, 'test/health-no-effects.mjs')).href,
    path.join(bundle, 'src/health.mjs'), '--state-dir', path.join(temp, 'state'), ...extraArgs], {
    cwd: temp, env: { HOME: home, USERPROFILE: home, CODEX_HOME: path.join(home, 'codex'),
      DIRECTOR_HEALTH_AUDIT: audit, ...(process.platform === 'win32' ? { SystemRoot: process.env.SystemRoot } : {}) },
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  let stdout = '', stderr = '', overflow = false;
  const take = (key, chunk) => {
    if (key === 'stdout') stdout += chunk; else stderr += chunk;
    if (stdout.length + stderr.length > 65536) { overflow = true; child.kill(); }
  };
  child.stdout.on('data', chunk => take('stdout', chunk));
  child.stderr.on('data', chunk => take('stderr', chunk));
  child.stdin.on('error', () => {});
  const timer = setTimeout(() => child.kill(), 9000);
  const result = new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('exit', (code, signal) => resolve({ code, signal }));
  });
  child.stdin.end(requests.map(value => typeof value === 'string' ? value : JSON.stringify(value)).join('\n') + '\n');
  try {
    const resultValue = await result;
    const evidence = JSON.parse(await fs.readFile(audit, 'utf8'));
    assert.equal(overflow, false);
    assert.equal(evidence.forbiddenEffectAttempts, 0);
    assert(!evidence.environmentKeys.includes('OPENAI_API_KEY'));
    return { ...resultValue, stdout, stderr, evidence };
  } finally { clearTimeout(timer); child.kill(); }
}

test('packaged genuine SDK hello/inspect/shutdown needs no runtime, account, network, or source checkout', async () => {
  const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'director-sdk-health-'));
  try {
    const bundle = path.join(temp, 'app-resource', 'sdk-readiness');
    const manifest = await packageReadiness(bundle);
    assert.equal(manifest.sdkVersion, pinned.version);
    assert.equal(manifest.officialRuntimeIncluded, false);
    assert.deepEqual(manifest.files.map(value => value.path), resourceFiles);
    await assert.rejects(fs.stat(path.join(bundle, 'test')));
    await assert.rejects(fs.stat(path.join(bundle, 'node_modules/@openai/codex')));
    const result = await worker(bundle, temp, ['hello', 'inspect', 'shutdown'].map(request));
    assert.equal(result.code, 0, result.stderr);
    assert.equal(result.stderr, '');
    const frames = result.stdout.trim().split('\n').map(JSON.parse);
    assert.equal(frames.length, 3);
    for (const frame of frames) {
      assert.equal(frame.ok, true);
      assert.equal(frame.health.sdkImported, true);
      assert.equal(frame.health.executeCalls, 0);
      assert.equal(frame.health.accountChecked, false);
      assert.equal(frame.health.networkTested, false);
    }
    assert.equal(frames[0].result.capabilities.execute, 'disabled');
    assert.equal(frames[1].result.offlineFixtureAvailable, false);
    assert.deepEqual(await fs.readdir(path.join(temp, 'state')), []);
    assert.deepEqual(await fs.readdir(path.join(temp, 'isolated-home')), []);
  } finally { await fs.rm(temp, { recursive: true, force: true }); }
});

test('production health rejects execute, resume, fixture args, malformed/out-of-order and oversized requests', async () => {
  for (const [requests, extraArgs] of [
    [[request('execute', 0)], []], [[request('resumeThread', 0)], []],
    [[request('inspect', 0)], []], [['{invalid'], []], [['x'.repeat(8193)], []],
    [[request('hello', 0)], ['--offline-fixture', 'happy']],
    [[{ ...request('hello', 0), params: { offlineFixture: 'happy' } }], []],
  ]) {
    const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'director-sdk-health-negative-'));
    try {
      const result = await worker(root, temp, requests, extraArgs);
      assert.notEqual(result.code, 0);
      assert.equal(result.stdout, '');
      assert.deepEqual(await fs.readdir(path.join(temp, 'isolated-home')), []);
    } finally { await fs.rm(temp, { recursive: true, force: true }); }
  }
});

test('packaging refuses unexpected files instead of shipping stale runtimes or fixtures', async () => {
  const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'director-sdk-package-negative-'));
  try {
    await fs.writeFile(path.join(temp, 'unexpected-runtime'), 'synthetic');
    await assert.rejects(packageReadiness(temp), /Unexpected resource/);
  } finally { await fs.rm(temp, { recursive: true, force: true }); }
});
