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
// Both send confirmation and passive recovery read the same logical message.
// ChatGPT can nest all three user selectors and append collapsed-message UI
// (ellipsis / Show more). Only the known plain-text body is the sent prompt;
// never trim, substring-match, or remove characters from that body.
export function readOwnedConversationDom(expected) {
  const empty={url:location.href,userCount:0,promptMatched:false,images:[],imageCandidates:0,selectors:0};
  // SPA navigation can retain an old, unrendered main beside the live thread.
  // Select exactly one rendered main, independently of tab visibility. A
  // background tab is still readable; two live mains remain ambiguous.
  const mains=[...document.querySelectorAll('main')].filter(n=>{
    if(n.getClientRects().length===0||n.closest('[hidden],[aria-hidden="true"]'))return false;
    const style=getComputedStyle(n);
    return style.display!=='none'&&!['hidden','collapse'].includes(style.visibility);
  });
  if(mains.length!==1)return empty;
  const main=mains[0],userSelector='[data-message-author-role="user"],[data-turn="user"],.bg-user-message';
  const matches=[...main.querySelectorAll(userSelector)];
  const users=matches.filter(n=>!matches.some(parent=>parent!==n&&parent.contains(n)));
  const result={...empty,userCount:users.length};
  if(users.length!==1)return result;
  const user=users[0],bubbles=[user,...user.querySelectorAll('.bg-user-message')].filter(n=>n.matches('.bg-user-message'));
  if(bubbles.length>1)return result;
  const scope=bubbles[0]||user;
  const bodies=[scope,...scope.querySelectorAll('.whitespace-pre-wrap')].filter(n=>n.matches('.whitespace-pre-wrap')&&!n.closest('button,[contenteditable],textarea'));
  const body=bodies.length===1?bodies[0]:bodies.length===0?scope:null;
  // This adapter sends plain text. Unknown structured/rendered bodies stay held.
  if(!body||![...body.childNodes].every(n=>n.nodeType===3))return result;
  result.promptMatched=typeof expected==='string'&&expected.length>0&&body.textContent===expected;
  if(!result.promptMatched)return result;
  const followsUser=n=>!user.contains(n)&&!n.closest(userSelector)&&!n.closest('nav,aside,[role="dialog"],[contenteditable]')
    &&(user.compareDocumentPosition(n)&5)===4;
  const candidates=[...new Set(main.querySelectorAll('[data-testid="generated-image-gallery"] img,button[data-testid="generated-image-preview"] img'))].filter(followsUser);
  result.imageCandidates=candidates.length;
  result.images=candidates.filter(n=>n.complete&&n.naturalWidth>128&&n.naturalHeight>128);
  result.selectors=[...main.querySelectorAll('button[aria-label]')].filter(n=>followsUser(n)&&/^(显示生成的图像|Show generated image) [0-9]+$/.test(n.getAttribute('aria-label')||'')).length;
  return result;
}
export function ownedConversationStateCode(text, generatingCode='false') {
  return `(()=>{const readOwnedConversationDom=${readOwnedConversationDom.toString()};const state=readOwnedConversationDom(${JSON.stringify(text)});return{url:state.url,userCount:state.userCount,promptMatched:state.promptMatched,count:state.images.length,imageCandidates:state.imageCandidates,selectors:state.selectors,generating:(${generatingCode})};})()`;
}
export async function observeOwnedSubmission(page, text) {
  for (let i = 0; i < 20; i++) {
    const result = await page.evaluate(ownedConversationStateCode(text));
    if (result?.userCount===1 && result.promptMatched===true && /^https:\/\/chatgpt\.com\/c\/[a-z0-9-]+$/.test(result.url || '')) return { confirmed:true, conversationUrl:result.url, promptSha256:hash(text) };
    if (i < 19) await page.sleep(0.25);
  }
  throw fail('SENT_PROMPT_UNCONFIRMED');
}
