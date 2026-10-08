// Trusted Rust creates this configuration after native readiness. The UI never
// supplies a worker, SDK implementation, runtime wrapper or credential content.
import { createHash } from 'node:crypto';
import { promises as fs } from 'node:fs';
import path from 'node:path';
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const check = (ok, message) => { if (!ok) throw new Error(`SDK_NATIVE_IDENTITY: ${message}`); };
export function nativeRuntime(config, stateDirectory) {
  check(path.isAbsolute(stateDirectory) && config?.version === 'sdk-native-config/v1', 'explicit native configuration required');
  const identity = config.identity;
  check(identity?.version === 'official-sdk-identity/v2' && identity.provider === 'codex_sdk'
    && identity.permissionProfile === 'sdk-text-only-native/v1', 'unsupported identity or permission profile');
  for (const key of ['runtimeExecutable', 'supervisorExecutable', 'credentialHome', 'codexHome']) check(path.isAbsolute(identity[key] ?? ''), key);
  check(Object.keys(config).sort().join(',') === 'identity,version', 'unexpected configuration fields');
  return Object.freeze({ identity, async create(directory, record) {
    check(path.dirname(directory) === stateDirectory && /^[A-Za-z0-9_-]{1,128}$/.test(path.basename(directory)), 'attempt directory');
    check(JSON.stringify(record.attempt.runtimeIdentity) === JSON.stringify(identity), 'frozen SDK identity changed');
    check(hash(await fs.readFile(identity.supervisorExecutable)) === identity.supervisorSha256, 'supervisor changed');
    check(hash(await fs.readFile(identity.runtimeExecutable)) === identity.runtimeSha256, 'official runtime changed');
    const root = path.join(directory, 'native-runtime');
    await fs.mkdir(root, { mode: 0o700 });
    const work = path.join(root, 'work'), temp = path.join(root, 'tmp');
    await fs.mkdir(work); await fs.mkdir(temp);
    await fs.writeFile(path.join(work, '.director-text-root'), 'director SDK text execution\n', { flag: 'wx', mode: 0o600 });
    const plan = { version: 'sdk-native-runtime/v1', attemptId: record.attempt.attemptId,
      identity, model: record.attempt.request.model, reasoning: record.attempt.request.reasoning,
      compiledPromptSha256: record.compiledPromptSha256,
      compiledSchemaSha256: record.compiledSchemaSha256,
      runtimeSchemaSha256: hash(JSON.stringify(JSON.parse(record.compiledSchema))),
      semanticRequestHash: record.attempt.semanticRequestHash,
      executionFingerprint: record.attempt.executionFingerprint,
      workingDirectory: work, tempDirectory: temp };
    const bytes = JSON.stringify(plan), file = path.join(root, 'runtime-plan.json');
    const handle = await fs.open(file, 'wx', 0o600);
    try { await handle.writeFile(bytes); await handle.sync(); } finally { await handle.close(); }
    // Preserve only the account location, not credential bytes or host config.
    // The native wrapper ignores user configuration and owns the runtime tree.
    const env = { DIRECTOR_SDK_RUNTIME_PLAN: file, DIRECTOR_SDK_RUNTIME_PLAN_SHA256: hash(bytes),
      USERPROFILE: identity.credentialHome, HOME: identity.credentialHome,
      TEMP: temp, TMP: temp, TMPDIR: temp };
    if (process.platform === 'win32') {
      check(process.env.SystemRoot && path.isAbsolute(process.env.SystemRoot), 'Windows system directory');
      env.SystemRoot = process.env.SystemRoot; env.WINDIR = process.env.SystemRoot;
    }
    return { executable: identity.supervisorExecutable, workingDirectory: work, temporaryDirectory: temp, env };
  } });
}
