// Synthetic subprocess only. No network libraries, credentials, or actual runtime calls.
import fs from 'node:fs';
const args = process.argv.slice(2);
const arg = flag => args[args.indexOf(flag) + 1];
let prompt = '';
for await (const chunk of process.stdin) prompt += chunk;
const scenario = process.env.DIRECTOR_FIXTURE_SCENARIO;
fs.writeFileSync(process.env.DIRECTOR_FIXTURE_CAPTURE, JSON.stringify({ args, prompt, schema: JSON.parse(fs.readFileSync(arg('--output-schema'), 'utf8')), env: process.env, pid: process.pid }), { mode: 0o600 });
const event = value => process.stdout.write(JSON.stringify(value) + '\n');
const thread = { type: 'thread.started', thread_id: 'synthetic-thread-1' };
const start = { type: 'turn.started' };
let fixtureText = '{"ok":true,"text":"合成测试"}';
if (scenario === 'rust-domain') {
  const request = JSON.parse(prompt);
  const input = request.input;
  if (request.stage === 'assetVisualFacts') {
    fixtureText = JSON.stringify(input.visualFactRepair ? {
      schemaVersion: 'asset-visual-fact-repair/v1', originalRawSha256: input.visualFactRepair.originalRawSha256,
      corrections: [{ factIndex: 0, action: 'UNRESOLVED', variantId: null, reason: '合成原文证据不足，保留待确认' }],
    } : {
      schemaVersion: 'asset-visual-facts-wire/v1', coveredPacketRef: input.coveredPacketRef,
      facts: [{ assetId: 'subject', variantId: null, attribute: 'hair.style', value: '局部变化', scope: 'TRANSITION', claim: 'SOURCE', sourceRefs: ['r1'] }], issues: [],
    });
  } else fixtureText = JSON.stringify(input.sdkFixtureResponse ?? { ok: true, text: '合成测试' });
}
const text = { type: 'item.completed', item: { id: 'synthetic-item-1', type: 'agent_message', text: scenario === 'invalid-text-json' ? '{not repaired' : fixtureText } };
const complete = { type: 'turn.completed', usage: { input_tokens: 3, cached_input_tokens: 0, output_tokens: 5, reasoning_output_tokens: 1 } };
const configurationNotice = { type: 'item.completed', item: { id: 'item_0', type: 'error', message: 'Codex is ignoring 1 unrecognized configuration setting. Check for typos or deprecated settings.\n  session-flags: `tools.view_image` is ignored.' } };
if (scenario === 'configuration-notice') { event(thread); event(configurationNotice); event(start); event(text); event(complete); }
else if (scenario === 'unstable-feature-notice') { event(thread); event({ ...configurationNotice, item: { ...configurationNotice.item, message: 'Under-development features enabled: skip_host_skill_discovery. Under-development features are incomplete and may behave unpredictably. To suppress this warning, set `suppress_unstable_features_warning = true` in synthetic config.toml.' } }); event(start); event(text); event(complete); }
else if (scenario === 'ordinary-error-item') { event(thread); event({ ...configurationNotice, item: { ...configurationNotice.item, message: 'A non-fatal diagnostic unrelated to configuration, 保留原始诊断。' } }); event(start); event(text); event(complete); }
else if (scenario === 'notice-missing-id') { event(thread); event({ ...configurationNotice, item: { type: 'error', message: 'A diagnostic without its required identity' } }); event(start); event(text); event(complete); }
else if (scenario === 'notice-too-long') { event(thread); event({ ...configurationNotice, item: { ...configurationNotice.item, message: '诊断'.repeat(683) } }); event(start); event(text); event(complete); }
else if (scenario === 'notice-extra-event-field') { event(thread); event({ ...configurationNotice, command: 'synthetic-unexecuted-command' }); event(start); event(text); event(complete); }
else if (scenario === 'tool-before-turn') { event(thread); event({ type: 'item.started', item: { id: 'tool-1', type: 'command_execution', command: 'synthetic-unexecuted-command', status: 'in_progress' } }); event(start); event(text); event(complete); }
else if (scenario === 'fatal-error-before-turn') { event(thread); event({ type: 'error', message: 'Synthetic fatal error is never a diagnostic item' }); }
else if (scenario === 'configuration-notice-only') { event(thread); event(configurationNotice); }
else if (scenario === 'configuration-notice-before-thread') { event(configurationNotice); event(thread); event(start); event(text); event(complete); }
else if (scenario === 'configuration-notice-malformed') { event(thread); event({ ...configurationNotice, item: { ...configurationNotice.item, text: 'must not be treated as agent text' } }); event(start); event(text); event(complete); }
else if (scenario === 'configuration-notice-after-turn') { event(thread); event(start); event(configurationNotice); event(text); event(complete); }
else if (scenario === 'agent-before-turn') { event(thread); event(text); event(start); event(complete); }
else if (scenario === 'out-of-order') { event(start); event(thread); event(text); event(complete); }
else if (scenario === 'malformed-json') { event(thread); event(start); process.stdout.write('{this is not runtime JSON}\n'); }
else if (scenario === 'early-complete') { event(thread); event(start); event(complete); event(text); }
else if (scenario === 'changed-thread') { event(thread); event({ ...thread, thread_id: 'unexpected-thread' }); }
else if (scenario === 'policy-violation') { event(thread); event(start); event({ type: 'item.started', item: { id: 'tool-1', type: 'command_execution', command: 'synthetic-unexecuted-command', status: 'in_progress' } }); }
else if (scenario === 'failed-after-text') { event(thread); event(start); event(text); event({ type: 'turn.failed', error: { message: 'Synthetic failure, do not retry' } }); }
else if (scenario === 'hang' || scenario === 'ignore-abort') {
  event(thread); event(start);
  if (scenario === 'ignore-abort') process.on('SIGTERM', () => {});
  setTimeout(() => process.exit(0), scenario === 'ignore-abort' ? 1600 : 30_000);
} else if (scenario === 'oversized-event') { event(thread); event(start); event({ ...text, item: { ...text.item, text: 'x'.repeat(1_048_577) } }); }
else if (scenario === 'same-item-mutated') { event(thread); event(start); event(text); event({ ...text, item: { ...text.item, text: '{"tampered":true}' } }); event(complete); }
else {
  event(thread); event(start); event(text); event(complete);
  if (scenario === 'duplicates') { event(text); event(complete); }
}
