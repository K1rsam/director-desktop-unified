import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import path from 'node:path';
import { recoverSavedImage } from './recover-saved-image.mjs';

test('current Chinese stop control prevents navigation before reading a saved result', async () => {
  const source = process.env.OPENCLI_CHECKOUT;
  if (!source) throw new Error('Set OPENCLI_CHECKOUT to the pinned checkout with jsdom installed');
  const { JSDOM } = createRequire(pathToFileURL(path.join(source, 'package.json')))('jsdom');
  const dom = new JSDOM('<form><button aria-label="停止"></button></form>', { url: 'https://chatgpt.com/c/current', runScripts: 'outside-only' });
  let navigations = 0;
  try {
    const page = { evaluate: async code => dom.window.eval(code), goto: async () => { navigations += 1; } };
    const receipt = { conversationUrl: 'https://chatgpt.com/c/saved-id', sha256: 'a'.repeat(64), bytes: 100, originalBytesVerified: true };
    await assert.rejects(recoverSavedImage(page, receipt), /Active or unverified page/);
    assert.equal(navigations, 0);
  } finally { dom.window.close(); }
});
