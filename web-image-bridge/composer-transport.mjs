// Owned ChatGPT composer operations. Preparation never clicks or presses Enter.
import fs from 'node:fs';
import { createHash } from 'node:crypto';
export const OWNED_SEND_PROTOCOL = 'owned-composer-send/v2';
export const composerAdapterSha256 = createHash('sha256').update(fs.readFileSync(new URL(import.meta.url))).digest('hex');
const validToken = value => typeof value === 'string' && /^[a-zA-Z0-9_-]{1,100}$/.test(value);
const fail = code => { const e = new Error(code); e.code = code; return e; };
const hash = value => createHash('sha256').update(value).digest('hex');
const visible = `const visible=n=>n instanceof HTMLElement&&n.getClientRects().length>0&&!n.closest('[hidden],[aria-hidden="true"]')&&getComputedStyle(n).display!=='none'&&getComputedStyle(n).visibility!=='hidden';`;
const key = token => `__director_owned_composer_${token}`;
// Serialize only the plain ProseMirror paragraph structure produced by fillText.
// CSS-derived innerText adds visual margins; textContent loses paragraph breaks.
// Unknown markup and embedded chips are held, never normalized or removed.
export function logicalComposerText(composer) {
  if (!composer || composer.querySelector('[contenteditable="false"]')) return { ok:false };
  if (!composer.isContentEditable) {
    return ['TEXTAREA','INPUT'].includes(composer.tagName) ? { ok:true,text:String(composer.value??'') } : { ok:false };
  }
  const paragraphs = [];
  for (const paragraph of composer.childNodes) {
    if (paragraph.nodeType!==1 || paragraph.tagName!=='P') return { ok:false };
    const nodes = [...paragraph.childNodes]; let text = '';
    for (let index=0; index<nodes.length; index++) {
      const node=nodes[index];
      if (node.nodeType===3) { text+=node.data; continue; }
      if (node.nodeType!==1 || node.tagName!=='BR' || node.childNodes.length!==0) return { ok:false };
      const placeholder=node.getAttribute('class')==='ProseMirror-trailingBreak';
      if (placeholder) {
        // Only this one-node empty-paragraph caret placeholder has no text.
        if (nodes.length!==1 || index!==nodes.length-1 || [...node.attributes].some(a=>a.name!=='class')) return { ok:false };
      } else {
        if (node.hasAttribute('class')) return { ok:false };
        text+='\n';
      }
    }
    if (paragraph.getAttribute('data-empty-paragraph')==='true' && text!=='') return { ok:false };
    paragraphs.push(text);
  }
  return { ok:true,text:paragraphs.join('\n') };
}
const logicalTextCode = `const logicalComposerText=${logicalComposerText.toString()};`;
export function readOwnedComposerCode(text, token) {
  return `(()=>{${logicalTextCode}const s=globalThis[${JSON.stringify(key(token))}],expected=${JSON.stringify(text)};if(!s||s.clicked||s.prompt!==expected||!s.composer.isConnected)return{ok:false,reason:'composer_owner_changed'};const actual=logicalComposerText(s.composer);return{ok:actual.ok,token:s.token,prompt:actual.text};})()`;
}
export function prepareComposerCode(text, token) {
  if (!validToken(token) || typeof text !== 'string' || !text) throw fail('COMPOSER_REQUEST_INVALID');
  return `(()=>{${visible}const token=${JSON.stringify(token)},k=${JSON.stringify(key(token))};if(globalThis[k])throw Error('COMPOSER_OWNER_EXISTS');const nodes=[...new Set([...document.querySelectorAll('#prompt-textarea,[data-testid="prompt-textarea"],[contenteditable="true"][role="textbox"]')])].filter(visible);if(nodes.length!==1)return{ok:false,reason:'composer_not_unique'};const c=nodes[0];if(c.querySelector('[contenteditable="false"]'))return{ok:false,reason:'embedded_content_requires_review'};c.setAttribute('data-director-owned-composer',token);globalThis[k]={version:${JSON.stringify(OWNED_SEND_PROTOCOL)},token,prompt:${JSON.stringify(text)},composer:c,clicked:false,button:null};return{ok:true};})()`;
}
export function readyComposerCode(text, token) {
  return `(()=>{${visible}${logicalTextCode}const s=globalThis[${JSON.stringify(key(token))}],expected=${JSON.stringify(text)};if(!s||s.clicked||s.prompt!==expected||!s.composer.isConnected)return{ok:false,reason:'composer_owner_changed'};const c=s.composer,actual=logicalComposerText(c);if(!actual.ok||actual.text!==expected)return{ok:false,reason:'composer_text_mismatch'};const scope=c.closest('form')||c.parentElement;const labels=['Send prompt','Send message','Send','发送','发送消息','发送提示'];const buttons=[...new Set([...scope.querySelectorAll('button[data-testid="send-button"],#composer-submit-button,button[aria-label]')])].filter(b=>visible(b)&&!b.disabled&&b.getAttribute('aria-disabled')!=='true'&&(b.matches('button[data-testid="send-button"],#composer-submit-button')||labels.includes(b.getAttribute('aria-label')||'')));if(buttons.length!==1)return{ok:false,reason:'send_button_not_unique_or_ready'};s.button=buttons[0];return{ok:true,token:s.token,prompt:actual.text,url:location.href};})()`;
}
export async function prepareOwnedComposer(page, text, token) {
  if (!(await page.evaluate(prepareComposerCode(text, token)))?.ok) throw fail('COMPOSER_NOT_READY');
  await page.fillText(`[data-director-owned-composer="${token}"]`, text);
  const filled = await page.evaluate(readOwnedComposerCode(text,token));
  if (filled?.ok!==true || filled.token!==token || filled.prompt!==text) throw fail('COMPOSER_TEXT_UNCONFIRMED');
  for (let i = 0; i < 20; i++) {
    const ready = await page.evaluate(readyComposerCode(text, token));
    if (ready?.ok && ready.token === token && ready.prompt === text && ['https://chatgpt.com/', 'https://chatgpt.com/new'].includes(ready.url)) {
      return { version:OWNED_SEND_PROTOCOL, token, promptSha256:hash(text), adapterSha256:composerAdapterSha256 };
    }
    if (i < 19) await page.sleep(0.25);
  }
  throw fail('SEND_BUTTON_NOT_READY');
}
export function clickComposerCode(text, gate) {
  return `(()=>{${visible}${logicalTextCode}const s=globalThis[${JSON.stringify(key(gate.token))}],expected=${JSON.stringify(text)};if(!s||s.version!==${JSON.stringify(OWNED_SEND_PROTOCOL)}||s.token!==${JSON.stringify(gate.token)}||s.prompt!==expected)return{clicked:false,reason:'owner_missing'};if(s.clicked)return{clicked:true,recovered:true};const c=s.composer,b=s.button,actual=logicalComposerText(c);if(!c?.isConnected||!actual.ok||actual.text!==expected||!b?.isConnected||!visible(b)||b.disabled||b.getAttribute('aria-disabled')==='true')return{clicked:false,reason:'gate_changed'};s.clicked=true;s.clickedAt=Date.now();b.click();return{clicked:true,recovered:false};})()`;
}
export async function clickOwnedComposer(page, text, gate, intent) {
  if (gate?.version !== OWNED_SEND_PROTOCOL || gate.adapterSha256 !== composerAdapterSha256 || !validToken(gate.token)
      || gate.promptSha256 !== hash(text) || intent?.version !== 'owned-click-intent/v1' || intent.token !== gate.token || intent.promptSha256 !== gate.promptSha256) throw fail('CLICK_INTENT_UNBOUND');
  // Page/daemon may retry an exec after a lost ACK. The synchronous page-owned
  // token is consumed BEFORE click, so those retries cannot click again.
  const result = await page.evaluate(clickComposerCode(text, gate));
  if (result?.clicked !== true) throw fail('CLICK_RESULT_UNCONFIRMED');
  return result;
}
export async function observeOwnedSubmission(page, text) {
  for (let i = 0; i < 20; i++) {
    const result = await page.evaluate(`(()=>{let users=[...document.querySelectorAll('[data-message-author-role="user"]')];if(!users.length)users=[...document.querySelectorAll('[data-turn="user"],.bg-user-message')].filter(n=>!n.querySelector('[data-turn="user"],.bg-user-message'));return{url:location.href,texts:users.map(n=>n.innerText??n.textContent??'')};})()`);
    if (result?.texts?.length === 1 && result.texts[0] === text && /^https:\/\/chatgpt\.com\/c\/[a-z0-9-]+$/.test(result.url || '')) return { confirmed:true, conversationUrl:result.url, promptSha256:hash(text) };
    if (i < 19) await page.sleep(0.25);
  }
  throw fail('SENT_PROMPT_UNCONFIRMED');
}
