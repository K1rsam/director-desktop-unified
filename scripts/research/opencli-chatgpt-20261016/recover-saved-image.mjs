// Isolated candidate: read an already verified result; never submit a prompt.
import { pathToFileURL } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';

export async function recoverSavedImage(page, receipt, { timeoutMs = 30000, pollMs = 2000 } = {}) {
  const { conversationUrl, sha256, bytes, originalBytesVerified } = receipt;
  if (!/^https:\/\/chatgpt\.com\/c\/[a-z0-9-]+$/.test(conversationUrl || '')
      || !/^[a-f0-9]{64}$/.test(sha256 || '') || !Number.isSafeInteger(bytes) || bytes < 1
      || originalBytesVerified !== true) throw new Error('A stable, previously verified result is required');
  if (!Number.isFinite(timeoutMs) || timeoutMs < 1 || timeoutMs > 60000 || pollMs < 1) throw new Error('Invalid bounded recovery timing');
  // The caller binds an owned page. Do not navigate away from any active generation.
  const active = await page.evaluate(`!!document.querySelector('[data-testid="stop-button"],button[aria-label*="Stop generating"],button[aria-label*="停止生成"]')`);
  if (active !== false) throw new Error('Active or unverified page; no navigation');
  const attempts = [];
  for (let attempt = 0; attempt < 2; attempt += 1) {
    await page.goto(conversationUrl, { settleMs: 1200 });
    const start = Date.now();
    do {
      const state = await page.evaluate(`(async () => {
        const images = [...document.querySelectorAll('[data-testid="generated-image-gallery"] img')].filter(i => i.complete && i.naturalWidth > 128);
        const url = location.href;
        if (images.length !== 1) return { ready: false, url };
        const buffer = await (await fetch(images[0].currentSrc || images[0].src)).arrayBuffer();
        const sha256 = [...new Uint8Array(await crypto.subtle.digest('SHA-256', buffer))].map(b => b.toString(16).padStart(2, '0')).join('');
        return { ready: true, url, sha256, bytes: buffer.byteLength, userTurns: document.querySelectorAll('[data-turn=user],[data-message-author-role=user],.bg-user-message').length };
      })()`);
      if (state?.url !== conversationUrl) throw new Error('Conversation changed; no fallback navigation or submission');
      if (state.ready) {
        if (state.sha256 !== sha256 || state.bytes !== bytes || state.userTurns !== 1) throw new Error('Original identity mismatch; no automatic retry');
        attempts.push({ attempt: attempt + 1, status: 'matched' });
        return { noSubmission: true, exactOriginalBytesMatch: true, attempts };
      }
      await page.sleep(pollMs / 1000);
    } while (Date.now() - start < timeoutMs);
    attempts.push({ attempt: attempt + 1, status: 'read_timeout' });
  }
  throw new Error('Original result unavailable after two read attempts; preserve receipt and stop');
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const [source, receiptPath, profile, ownedPage] = process.argv.slice(2);
  if (!source || !receiptPath || !profile || !ownedPage) throw new Error('Usage: node recover-saved-image.mjs CHECKOUT RECEIPT PROFILE OWNED_PAGE');
  if (execFileSync('git', ['-C', source, 'rev-parse', 'HEAD'], { encoding: 'utf8', windowsHide: true }).trim() !== 'b9213a15d63df578612ddaedc4e3b5c3428d5305') throw new Error('Source pin mismatch');
  const { Page } = await import(pathToFileURL(path.join(source, 'dist/src/browser/page.js')).href);
  const page = new Page('site:chatgpt', undefined, profile, 'background', 'adapter', 'persistent');
  page.setActivePage(ownedPage);
  console.log(JSON.stringify(await recoverSavedImage(page, JSON.parse(fs.readFileSync(receiptPath, 'utf8')))));
}
