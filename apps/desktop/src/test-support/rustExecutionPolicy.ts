import { readFileSync } from 'node:fs';

/** Read the complete advertised Rust catalogue so a newly registered action
 * cannot silently invalidate the native policy while a hand-picked mock passes. */
export function rustStandardActionCatalogue(): string[] {
  const source = readFileSync(new URL('../../src-tauri/src/execution_policy.rs', import.meta.url), 'utf8');
  return ['OFFLINE_ACTIONS', 'STANDARD_ACTIONS', 'STAGE3_ACTIONS'].flatMap(name => {
    const body = new RegExp(`pub const ${name}: &[\\s\\S]*?= &\\[([\\s\\S]*?)\\];`).exec(source)?.[1];
    if (body === undefined) throw new Error(`Missing Rust action catalogue: ${name}`);
    return [...body.matchAll(/"([^"\r\n]+)"/g)].map(match => match[1]);
  });
}
