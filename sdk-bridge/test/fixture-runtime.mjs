import { promises as fs } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
const source = fileURLToPath(new URL('./fixtures/fake-codex.mjs', import.meta.url));
export const SCENARIOS = Object.freeze(['rust-domain', 'happy', 'duplicates', 'out-of-order', 'malformed-json', 'early-complete', 'changed-thread', 'policy-violation', 'failed-after-text', 'hang', 'ignore-abort', 'invalid-text-json', 'oversized-event', 'same-item-mutated', 'configuration-notice', 'configuration-notice-only', 'configuration-notice-before-thread', 'configuration-notice-malformed', 'configuration-notice-after-turn', 'agent-before-turn']);
export function offlineFixture(scenario) {
  if (!SCENARIOS.includes(scenario)) throw new Error('Unknown offline fixture scenario');
  return Object.freeze({
    async create(directory) {
      const isolated = path.join(directory, 'synthetic-runtime');
      await fs.mkdir(isolated, { recursive: true, mode: 0o700 });
      const home = path.join(isolated, 'home');
      const work = path.join(isolated, 'work');
      await fs.mkdir(home, { mode: 0o700 }); await fs.mkdir(work, { mode: 0o700 });
      const script = path.join(isolated, 'fake-codex.mjs');
      await fs.copyFile(source, script);
      // The official SDK spawns an executable with argv beginning `exec`.
      // Use the installed native Node executable on every OS, including Windows,
      // and redirect only this synthetic child's entry point before main runs.
      // No shell/.cmd wrapper, extra subprocess, real Codex runtime, or SDK patch.
      const bootstrap = path.join(isolated, 'fake-codex-bootstrap.cjs');
      const code = `const Module = require('node:module');
Module.runMain = () => {
  process.argv = [process.execPath, ${JSON.stringify(script)}, 'exec', ...process.argv.slice(2)];
  import(${JSON.stringify(pathToFileURL(script).href)}).catch(error => { console.error(error); process.exitCode = 1; });
};
`;
      await fs.writeFile(bootstrap, code, { mode: 0o600 });
      return { executable: process.execPath, workingDirectory: work, env: { HOME: home, CODEX_HOME: home, TMPDIR: isolated, NODE_OPTIONS: `--require=${JSON.stringify(bootstrap)}`, DIRECTOR_FIXTURE_SCENARIO: scenario, DIRECTOR_FIXTURE_CAPTURE: path.join(directory, 'fixture-invocation.json') } };
    },
  });
}
