// Explicit operator-only live acceptance. Not matched by *.test.mjs or CI.
// Exactly one synthetic SDK turn; existing directories never acquire new budget.
import { promises as fs } from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { OfficialSdkBridge, PROTOCOL_VERSION } from '../src/bridge.mjs';
import { nativeRuntime } from '../src/native-runtime.mjs';
import { pinned, sdkRoot } from '../scripts/package-readiness.mjs';
const sha = value => createHash('sha256').update(value).digest('hex');
const canonical = value => Array.isArray(value) ? value.map(canonical) : value && typeof value === 'object'
  ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value;
const jsonHash = value => sha(JSON.stringify(canonical(value)));
const check = (value, code) => { if (!value) throw new Error(code); };
const args = process.argv.slice(2), options = {};
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--execute-once') options.executeOnce = true;
  else if (args[i] === '--observe-only') options.observeOnly = true;
  else if (['--state-dir', '--runtime', '--supervisor', '--case-id'].includes(args[i]) && args[i + 1]) options[args[i].slice(2)] = args[++i];
  else throw new Error('LIVE_SMOKE_ARGUMENTS');
}
check(process.platform === 'win32', 'WINDOWS_NATIVE_ONLY');
check(path.isAbsolute(options['state-dir'] ?? ''), 'DEDICATED_ABSOLUTE_STATE_REQUIRED');
check(options.executeOnce !== options.observeOnly, 'CHOOSE_EXECUTE_ONCE_OR_LOCAL_OBSERVE');
const state = options['state-dir'];
const attemptId = options['case-id'] ?? 'sdk-live-synthetic-once';
check(/^[A-Za-z0-9_-]{1,128}$/.test(attemptId), 'EXPLICIT_SYNTHETIC_CASE_ID_INVALID');
const message = (method, requestId, params) => ({ protocolVersion: PROTOCOL_VERSION, requestId, attemptId: ['hello', 'inspect', 'shutdown'].includes(method) ? null : attemptId, method, ...(params ? { params } : {}) });
async function localCommand(executable, argv) {
  const child = spawn(executable, argv, { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  let out = '', err = 0, exceeded = false;
  child.stdout.on('data', bytes => { if (out.length + bytes.length > 262144) { exceeded = true; child.kill(); } else out += bytes; });
  child.stderr.on('data', bytes => { err += bytes.length; if (err > 65536) { exceeded = true; child.kill(); } });
  const timer = setTimeout(() => child.kill(), 10000);
  try { const code = await new Promise((resolve, reject) => { child.on('error', reject); child.on('close', resolve); }); check(code === 0 && !exceeded, 'READONLY_LOCAL_COMMAND_FAILED'); return out; }
  finally { clearTimeout(timer); child.kill(); }
}
async function readonlyAccountAndModel(executable) {
  const version = (await localCommand(executable, ['--version'])).trim();
  check(version.startsWith('codex-cli '), 'OFFICIAL_RUNTIME_VERSION_REQUIRED');
  const help = await localCommand(executable, ['exec', '--help']);
  check(['--json', '--output-schema', '--ignore-user-config', '--ephemeral'].every(flag => help.includes(flag)), 'RUNTIME_EXEC_PROTOCOL_UNSUPPORTED');
  const child = spawn(executable, ['app-server', '--stdio', '-c', 'model_provider="openai"', '-c', 'chatgpt_base_url="https://chatgpt.com/backend-api"', '-c', 'features.plugins=false', '-c', 'features.apps=false', '-c', 'features.hooks=false', '-c', 'features.memories=false'], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  let buffer = '', bytes = 0, errorBytes = 0, id = 0;
  const pending = new Map();
  const fail = () => { for (const { reject } of pending.values()) reject(new Error('READONLY_ACCOUNT_RPC_FAILED')); pending.clear(); };
  child.on('error', fail); child.on('exit', fail);
  child.stderr.on('data', chunk => { errorBytes += chunk.length; if (errorBytes > 65536) { fail(); child.kill(); } });
  child.stdout.on('data', chunk => {
    bytes += chunk.length; if (bytes > 16777216 || buffer.length + chunk.length > 1048576) { fail(); child.kill(); return; }
    buffer += chunk.toString('utf8');
    let newline;
    while ((newline = buffer.indexOf('\n')) >= 0) {
      const line = buffer.slice(0, newline); buffer = buffer.slice(newline + 1);
      try { const response = JSON.parse(line), waiter = pending.get(response.id); if (waiter) { pending.delete(response.id); response.error ? waiter.reject(new Error('READONLY_ACCOUNT_RPC_REJECTED')) : waiter.resolve(response.result); } } catch { fail(); child.kill(); }
    }
  });
  const call = (method, params) => new Promise((resolve, reject) => { const current = ++id; pending.set(current, { resolve, reject }); child.stdin.write(JSON.stringify({ id: current, method, params }) + '\n'); });
  const timer = setTimeout(() => { fail(); child.kill(); }, 20000);
  try {
    await call('initialize', { clientInfo: { name: 'director-sdk-native-smoke', title: 'Synthetic SDK acceptance', version: '0.1.0' }, capabilities: { experimentalApi: false } });
    child.stdin.write(JSON.stringify({ method: 'initialized', params: {} }) + '\n');
    const account = (await call('account/read', { refreshToken: false })).account;
    check(account && typeof account === 'object', 'EXISTING_ACCOUNT_REQUIRED_NO_LOGIN');
    const model = (await call('model/list', { limit: 100, cursor: null, includeHidden: false })).data.find(row => (row.model ?? row.id) === 'gpt-6.1-sol');
    check(model && model.supportedReasoningEfforts.some(row => (row.reasoningEffort ?? row) === 'medium'), 'EXPLICIT_MODEL_REASONING_UNAVAILABLE');
    const limits = await call('account/rateLimits/read', {});
    const buckets = Object.values(limits.rateLimitsByLimitId ?? { codex: limits.rateLimits });
    for (const bucket of buckets.filter(Boolean)) {
      if (bucket.credits?.unlimited || bucket.credits?.hasCredits) continue;
      check(![bucket.primary, bucket.secondary].some(window => window?.usedPercent >= 100), 'EXISTING_ACCOUNT_USAGE_LIMIT');
    }
    return { version, accountFingerprint: jsonHash(account), model: 'gpt-6.1-sol', reasoning: 'medium', readonlyModelAvailable: true, refreshedToken: false };
  } finally {
    clearTimeout(timer); child.stdin.end(); child.kill();
    await new Promise(resolve => { if (child.exitCode !== null || child.signalCode !== null) resolve(); else child.once('close', resolve); });
  }
}
if (options.observeOnly) {
  const before = await fs.readFile(path.join(state, attemptId, 'events.ndjson'));
  const bridge = new OfficialSdkBridge({ stateDirectory: state });
  await bridge.initialize();
  try {
    const receipt = (await bridge.handle(message('observe', 'observe-only-1'))).result;
    check(receipt.state === 'transport_completed', 'LOCAL_RECEIPT_UNRESOLVED_NO_RETRY');
    check((await fs.readFile(path.join(state, attemptId, 'events.ndjson'))).equals(before), 'LOCAL_OBSERVE_MUTATED_EVENTS');
    console.log(JSON.stringify({ mode: 'observe-only', state: receipt.state, generationCalls: 0, remoteObservationCalls: 0, rawTextSha256: sha(receipt.rawResult.rawText) }));
  } finally { await bridge.disconnect(); }
} else {
  for (const key of ['runtime', 'supervisor']) check(path.isAbsolute(options[key] ?? ''), 'EXPLICIT_NATIVE_EXECUTABLES_REQUIRED');
  // Directory allocation is exclusive and persists before account/model work.
  // Prepare only the parent; never reuse an existing worker or its budget.
  await fs.mkdir(path.dirname(state), { recursive: true });
  await fs.mkdir(state);
  await fs.writeFile(path.join(state, 'authorized-budget.json'), JSON.stringify({ version: 'sdk-synthetic-live-budget/v1', caseId: attemptId, maximumExecuteCalls: 1, images: false, videos: false, productionProjects: false }), { flag: 'wx' });
  check(sha(await fs.readFile(path.join(sdkRoot, 'node_modules/@openai/codex-sdk/dist/index.js'))) === pinned.implementation, 'PINNED_SDK_IMPLEMENTATION_CHANGED');
  check(sha(await fs.readFile(path.join(sdkRoot, 'node_modules/@openai/codex-sdk/package.json'))) === pinned.package, 'PINNED_SDK_PACKAGE_CHANGED');
  const proof = await readonlyAccountAndModel(options.runtime);
  const bridgeBytes = await Promise.all(['src/main.mjs', 'src/bridge.mjs', 'src/native-runtime.mjs'].map(file => fs.readFile(path.join(sdkRoot, file), 'utf8')));
  const runtimeSha = sha(await fs.readFile(options.runtime));
  const home = process.env.USERPROFILE ?? process.env.HOME;
  check(path.isAbsolute(home ?? ''), 'EXPLICIT_EXISTING_ACCOUNT_HOME_REQUIRED');
  const identity = canonical({ version: 'official-sdk-identity/v2', provider: 'codex_sdk', sdkPackage: '@openai/codex-sdk', sdkVersion: pinned.version,
    sdkImplementationSha256: pinned.implementation, bridgeVersion: '0.1.0', bridgeSha256: sha(bridgeBytes.join('')), protocolVersion: PROTOCOL_VERSION,
    nodeExecutable: process.execPath, nodeSha256: sha(await fs.readFile(process.execPath)), nodeVersion: process.version,
    runtimePackageVersion: proof.version, runtimeIdentity: `official-codex-runtime/${proof.version}/${runtimeSha}`, permissionProfile: 'sdk-text-only-native/v1',
    accountFingerprint: proof.accountFingerprint, runtimeExecutable: options.runtime, runtimeSha256: runtimeSha,
    supervisorExecutable: options.supervisor, supervisorSha256: sha(await fs.readFile(options.supervisor)), resourceDirectory: sdkRoot,
    credentialHome: home, codexHome: process.env.CODEX_HOME ?? path.join(home, '.codex') });
  const schema = { type: 'object', properties: { ok: { type: 'boolean', const: true }, text: { type: 'string', const: 'SDK原生文本测试' } }, required: ['ok', 'text'], additionalProperties: false };
  const compiledPrompt = `独立合成 SDK 验收 ${attemptId}。不要读取文件、调用工具或媒体服务。只返回 JSON {"ok":true,"text":"SDK原生文本测试"}。`;
  const compiledSchema = JSON.stringify(canonical(schema));
  const request = { stage: 'sdkSmoke', skill: 'synthetic-sdk-smoke', model: proof.model, reasoning: proof.reasoning, input: { synthetic: true, caseId: attemptId }, schema, ruleSnapshot: { synthetic: true } };
  const semanticRequestHash = jsonHash({ stage: request.stage, skill: request.skill, input: request.input, schema, ruleSnapshot: request.ruleSnapshot });
  const executionFingerprint = jsonHash({ version: 'execution-fingerprint/v1', backend: 'codex_sdk', model: request.model, reasoning: request.reasoning, runtime: identity });
  const workflowId = `synthetic-sdk-smoke-${attemptId}`, logicalNodeId = `single-text-json-${attemptId}`;
  const attempt = { version: 'text-attempt/v2', attemptId, workflowId, logicalNodeId, backend: 'codex_sdk', request, semanticRequestHash, executionFingerprint, runtimeIdentity: identity,
    attemptEnvelopeHash: jsonHash({ version: 'attempt-identity/v1', attemptId, workflowId, logicalNodeId, semanticRequestHash, executionFingerprint }), dispatchStarted: false,
    state: 'prepared', threadId: null, turnId: null, lastSequence: 0, rawResult: null };
  const runtime = nativeRuntime({ version: 'sdk-native-config/v1', identity }, state);
  const bridge = new OfficialSdkBridge({ stateDirectory: state, nativeRuntime: runtime, limits: { executionMs: 120000 } });
  await fs.writeFile(path.join(state, 'frozen-request.json'), JSON.stringify(canonical({ attempt, compiledPrompt, compiledSchema })), { flag: 'wx' });
  await bridge.initialize();
  let receipt;
  try {
    await bridge.handle(message('execute', 'execute-once-1', { attempt, compiledPrompt, compiledSchema, compiledPromptSha256: sha(compiledPrompt), compiledSchemaSha256: sha(compiledSchema) }));
    await bridge.active.task;
    receipt = (await bridge.handle(message('observe', 'observe-current-1'))).result;
    await fs.writeFile(path.join(state, 'first-result.json'), JSON.stringify(receipt), { flag: 'wx' });
    check(receipt.state === 'transport_completed', 'SDK_SUBMISSION_UNRESOLVED_NO_RETRY');
    check(receipt.rawResult.parsedJson?.ok === true && receipt.rawResult.parsedJson.text === 'SDK原生文本测试', 'SYNTHETIC_RAW_OUTPUT_REJECTED_NO_RETRY');
    const processProof = JSON.parse(await fs.readFile(path.join(state, attemptId, 'native-runtime/process-receipt.json'), 'utf8'));
    check(processProof.tree.processTreeStopProven === true && processProof.tree.activeProcesses === 0 && processProof.exitCode === 0 && processProof.failure === null, 'NATIVE_PROCESS_COMPLETION_UNPROVEN');
  } finally { await bridge.disconnect(); }
  // Reconstructing the bridge never resumes a thread or dispatches a new turn.
  const before = await fs.readFile(path.join(state, attemptId, 'events.ndjson'));
  const reopened = new OfficialSdkBridge({ stateDirectory: state });
  await reopened.initialize();
  try {
    const restored = (await reopened.handle(message('observe', 'reopen-observe-1'))).result;
    check(restored.state === receipt.state && restored.rawResult.rawText === receipt.rawResult.rawText && (await fs.readFile(path.join(state, attemptId, 'events.ndjson'))).equals(before), 'REOPEN_RAW_RECEIPT_MISMATCH_NO_RETRY');
    const summary = { version: 'sdk-native-live-smoke/v1', sdkVersion: pinned.version, runtimeVersion: proof.version, model: proof.model, reasoning: proof.reasoning,
      executeCalls: 1, reopenedExecuteCalls: 0, images: 0, videos: 0, productionProjectReads: 0, state: restored.state, usage: receipt.usage,
      rawTextSha256: sha(receipt.rawResult.rawText), journalSha256: sha(before), threadIdPresent: !!receipt.threadId, turnId: null,
      processTreeStopProven: true, remoteObservationSupported: false, frozenIdentity: identity };
    await fs.writeFile(path.join(state, 'acceptance-receipt.json'), JSON.stringify(summary, null, 2), { flag: 'wx' });
    console.log(JSON.stringify({ ...summary, frozenIdentity: undefined }));
  } finally { await reopened.disconnect(); }
}
