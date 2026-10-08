// Two synthetic Node process trees only. No project, account, network, compiler
// or provider calls; this file can run through the pre-existing native runner.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { isAbsolute, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const self = fileURLToPath(import.meta.url);
const quoteProbe = '中文 空格 # % quote"尾\\';
const sleep = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
const [mode, ...args] = process.argv.slice(2);

if (mode?.startsWith('--fixture-')) {
  const [behavior, directory, argument] = args;
  assert.equal(argument, quoteProbe, 'Unicode, spaces, quotes and a trailing slash must round trip');
  const role = mode.slice('--fixture-'.length);
  if (role !== 'leaf') {
    const next = role === 'root' ? 'branch' : 'leaf';
    const child = spawn(process.execPath, [self, `--fixture-${next}`, behavior, directory, quoteProbe], {
      // A detached child remains in a non-breakaway Windows Job Object.
      detached: true, stdio: 'inherit', windowsHide: true,
    });
    await new Promise((resolve, reject) => { child.once('spawn', resolve); child.once('error', reject); });
    child.unref();
  }
  writeFileSync(join(directory, `${role}.json`), JSON.stringify({ pid: process.pid, role }));
  if (role === 'root') {
    const started = Date.now();
    while (!existsSync(join(directory, 'leaf.json'))) {
      assert.ok(Date.now() - started < 1500, 'synthetic leaf must start before the 2-second deadline');
      await sleep(10);
    }
    process.stdout.write('synthetic tree ready\n');
    process.stderr.write('synthetic stderr ready\n');
    if (behavior === 'normal') process.exit(0);
  }
  setInterval(() => {}, 1000);
} else {
  assert.equal(process.platform, 'win32', 'this regression requires Windows');
  assert.equal(mode, '--runner');
  const [runner, evidenceFlag, evidence] = args;
  assert.equal(evidenceFlag, '--evidence-dir');
  assert.ok(isAbsolute(runner) && existsSync(runner), 'an existing absolute runner is required');
  assert.ok(isAbsolute(evidence) && !existsSync(evidence), 'the evidence directory must be absolute and new');
  mkdirSync(evidence, { recursive: true });
  const tests = [];
  for (const behavior of ['timeout', 'normal']) {
    const directory = join(evidence, `fixture-${behavior}`);
    mkdirSync(directory);
    const output = join(evidence, `runner-${behavior}`);
    const commandArgs = ['--timeout-seconds', '2', output, '--', process.execPath,
      self, '--fixture-root', behavior, directory, quoteProbe];
    const launched = spawn(runner, commandArgs, { stdio: 'ignore', windowsHide: true });
    let launchError;
    launched.once('error', error => { launchError = error; });
    // The MSIX-safe launcher may exit before its Explorer-context child. The
    // receipt, not the initial process exit, is the completion boundary.
    const started = Date.now();
    const receiptPath = join(output, 'receipt.json');
    while (!existsSync(receiptPath)) {
      if (launchError) throw launchError;
      assert.ok(Date.now() - started < 20000, `${behavior}: runner receipt timed out`);
      await sleep(25);
    }
    const receipt = JSON.parse(readFileSync(receiptPath, 'utf8'));
    assert.equal(receipt.unpackagedRunner, true);
    assert.equal(receipt.sharedWriteProbeVerified, true);
    assert.equal(receipt.timeoutSeconds, 2);
    assert.equal(receipt.processTreeContainment, 'windows-job-object-at-creation');
    assert.equal(receipt.processTreeEmptyVerified, true);
    // Windows may add a console helper for a detached Node child. All three
    // recorded fixture PIDs, and every helper in the job, must be gone.
    assert.ok(receipt.jobProcessesSeen >= 3);
    assert.equal(receipt.jobTerminationIssued, true);
    assert.equal(receipt.timedOut, behavior === 'timeout');
    assert.equal(receipt.success, behavior === 'normal');
    assert.equal(receipt.exitCode, behavior === 'normal' ? 0 : 124);
    assert.ok(readFileSync(join(output, 'stdout.log'), 'utf8').includes('synthetic tree ready'));
    assert.ok(readFileSync(join(output, 'stderr.log'), 'utf8').includes('synthetic stderr ready'));
    const pids = ['root', 'branch', 'leaf'].map(role => JSON.parse(readFileSync(join(directory, `${role}.json`), 'utf8')).pid);
    for (const pid of pids) {
      let running = true;
      try { process.kill(pid, 0); } catch (error) { if (error.code === 'ESRCH') running = false; else throw error; }
      assert.equal(running, false, `${behavior}: owned synthetic PID ${pid} survived cleanup`);
    }
    tests.push({ name: `${behavior}-three-process-tree`, passed: true, receiptPath,
      jobProcessesSeen: receipt.jobProcessesSeen, processTreeEmptyVerified: true,
      timedOut: receipt.timedOut, exitCode: receipt.exitCode, pidsNoLongerRunning: pids });
  }
  const receipt = { schemaVersion: 'windows-native-runner-regression/v1', platform: process.platform,
    testsPassed: tests.length, tests, runnerSha256: createHash('sha256').update(readFileSync(runner)).digest('hex'),
    modelCalls: 0, productionProjectAccessed: false };
  writeFileSync(join(evidence, 'receipt.json'), JSON.stringify(receipt, null, 2));
  console.log(JSON.stringify(receipt));
}
