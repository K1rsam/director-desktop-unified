// ChatGPT web transport only. No image API, cookie export, video or automatic resend.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { Page } from './vendor/opencli/dist/src/browser/page.js';
import { setDaemonCommandTimeoutSeconds } from './vendor/opencli/dist/src/browser/daemon-client.js';
import { uploadChatGPTImages } from './vendor/opencli/clis/chatgpt/utils.js';
import { OWNED_SEND_PROTOCOL, composerAdapterSha256, logicalComposerText, prepareOwnedComposer, clickOwnedComposer, observeOwnedSubmission, readOwnedConversationDom, ownedConversationStateCode } from './composer-transport.mjs';

const sha = bytes => createHash('sha256').update(bytes).digest('hex');
export const requestHash = request => sha(JSON.stringify(request));
export const stableConversation = url => /^https:\/\/chatgpt\.com\/c\/[a-z0-9-]+$/.test(url || '');
const imageRatios = new Set(['1:1','16:9','9:16','4:3','3:4','3:2','2:3','21:9']);
export function validateRequest(r, verifyFiles = true) {
  if (!r || !/^[a-zA-Z0-9_-]{1,100}$/.test(r.requestId) || r.kind !== 'image' || r.model !== 'chatgpt-image'
      || typeof r.prompt !== 'string' || !r.prompt.trim() || r.prompt.length > 80000 || !Array.isArray(r.references) || r.references.length > 10
      || (r.ratio !== undefined && !imageRatios.has(r.ratio))) throw new Error('INVALID_REQUEST');
  if (!verifyFiles) return;
  for (const ref of r.references) {
    if (ref.kind !== 'image' || !path.isAbsolute(ref.path) || !/^[a-f0-9]{64}$/.test(ref.sha256)
        || fs.lstatSync(ref.path).isSymbolicLink() || fs.statSync(ref.path).size > 20 * 1024 * 1024
        || sha(fs.readFileSync(ref.path)) !== ref.sha256) throw new Error('REFERENCE_CHANGED');
  }
}
export function webImagePrompt(request) {
  validateRequest(request,false);
  const canvas = request.ratio ? `Use a ${request.ratio} aspect-ratio canvas (width:height); compose all requested views inside that single image. ` : '';
  return 'Generate one image only. ' + canvas + request.prompt;
}
export function checkIdentity(saved, request) {
  if (saved.requestHash !== requestHash(request)) throw new Error('REQUEST_IDENTITY_CHANGED');
}
function atomic(file, data) {
  const temp = file + '.' + process.pid + '.tmp';
  fs.writeFileSync(temp, JSON.stringify(data, null, 2), { mode: 0o600 }); fs.renameSync(temp, file);
}
function error(code, unknown = false) { const e = new Error(code); e.code = code; e.unknown = unknown; return e; }
// ChatGPT retains hidden stop controls after the image is complete. They are
// not a running generation; a visible control in the owned page still blocks.
export const activeCode = `[...document.querySelectorAll('[data-testid="stop-button"],button[aria-label="停止"],button[aria-label="Stop"],button[aria-label*="停止生成"],button[aria-label*="Stop generating"]')].some(button=>button.getClientRects().length>0&&!button.closest('[hidden],[aria-hidden="true"]')&&getComputedStyle(button).visibility!=='hidden')`;
const session = profile => new Page('site:chatgpt', undefined, profile, 'background', 'adapter', 'persistent');

// The daemon accepts at most 1 MiB per command. The upstream fallback puts a
// complete image into one evaluate call; a normal PNG can exceed that limit.
// Keep the pinned upstream code intact and supply its file-input hook with a
// bounded, hash-checked browser upload instead. This never sends the prompt.
export async function attachReferenceFiles(page, references, token, chunkBytes = 128 * 1024) {
  if (!Array.isArray(references) || !references.length || references.length>10 || !/^[a-zA-Z0-9_-]{1,100}$/.test(token) || !Number.isInteger(chunkBytes) || chunkBytes < 1 || chunkBytes > 128 * 1024) throw error('REFERENCE_UPLOAD_CONFIG');
  const key = `__director_reference_${token}`;
  const files = references.map(ref => {
    const bytes = fs.readFileSync(ref.path);
    if (bytes.length > 20 * 1024 * 1024 || sha(bytes) !== ref.sha256) throw error('REFERENCE_CHANGED');
    return { name:path.basename(ref.path), sha256:ref.sha256, bytes, mime:/\.png$/i.test(ref.path)?'image/png':/\.webp$/i.test(ref.path)?'image/webp':/\.gif$/i.test(ref.path)?'image/gif':/\.heic$/i.test(ref.path)?'image/heic':/\.heif$/i.test(ref.path)?'image/heif':'image/jpeg' };
  });
  const meta = files.map(({name,sha256,bytes,mime})=>({name,sha256,size:bytes.length,mime}));
  const initialized = await page.evaluate(`(()=>{const key=${JSON.stringify(key)},meta=${JSON.stringify(meta)};const old=globalThis[key];if(old){if(JSON.stringify(old.meta)!==JSON.stringify(meta))throw Error('REFERENCE_UPLOAD_IDENTITY');return{ok:true,phase:old.phase};}globalThis[key]={meta,parts:meta.map(()=>[]),files:[],phase:'collecting'};return{ok:true,phase:'collecting'};})()`);
  if (!initialized?.ok) throw error('REFERENCE_UPLOAD_UNCONFIRMED');
  if (initialized.phase === 'attached') return { count: files.length };
  if (initialized.phase !== 'collecting') throw error('REFERENCE_UPLOAD_UNCONFIRMED');
  for (let fileIndex=0; fileIndex<files.length; fileIndex++) {
    const file=files[fileIndex];
    for (let offset=0,index=0; offset<file.bytes.length; offset+=chunkBytes,index++) {
      const data=file.bytes.subarray(offset,offset+chunkBytes).toString('base64');
      const result=await page.evaluate(`(()=>{const s=globalThis[${JSON.stringify(key)}];if(!s||s.phase!=='collecting')throw Error('REFERENCE_UPLOAD_PHASE');const a=s.parts[${fileIndex}],i=${index},b=${JSON.stringify(data)};if(i<a.length){if(a[i]!==b)throw Error('REFERENCE_UPLOAD_IDENTITY');}else{if(i!==a.length)throw Error('REFERENCE_UPLOAD_ORDER');a.push(b);}return{ok:true,index:i};})()`);
      if (!result?.ok || result.index!==index) throw error('REFERENCE_UPLOAD_UNCONFIRMED');
    }
  }
  const attached=await page.evaluate(`(async()=>{const key=${JSON.stringify(key)},s=globalThis[key];if(!s)throw Error('REFERENCE_UPLOAD_MISSING');if(s.phase==='attached')return{ok:true,count:s.meta.length};if(s.phase!=='collecting')throw Error('REFERENCE_UPLOAD_UNCONFIRMED');const hash=async b=>[...new Uint8Array(await crypto.subtle.digest('SHA-256',b))].map(v=>v.toString(16).padStart(2,'0')).join('');const files=[];for(let i=0;i<s.meta.length;i++){const m=s.meta[i],chunks=s.parts[i].map(v=>Uint8Array.from(atob(v),c=>c.charCodeAt(0)));const blob=new Blob(chunks,{type:m.mime});if(blob.size!==m.size||await hash(await blob.arrayBuffer())!==m.sha256)throw Error('REFERENCE_UPLOAD_BYTES');files.push(new File([blob],m.name,{type:m.mime}));}const inputs=[...document.querySelectorAll('input[type="file"]')];const input=inputs.find(n=>/image|\.png|\.jpg|\.jpeg/i.test(n.accept||''))||inputs.find(n=>!(n.accept||''));if(!input)throw Error('REFERENCE_FILE_INPUT_MISSING');const dt=new DataTransfer();for(const file of files)dt.items.add(file);s.phase='invoking';input.files=dt.files;const propsKey=Object.keys(input).find(k=>k.startsWith('__reactProps$'));if(propsKey&&typeof input[propsKey]?.onChange==='function'){const nativeEvent=new Event('change',{bubbles:true});input[propsKey].onChange({target:input,currentTarget:input,nativeEvent,preventDefault(){},stopPropagation(){},isDefaultPrevented(){return false},isPropagationStopped(){return false},persist(){}});}else{input.dispatchEvent(new Event('input',{bubbles:true}));input.dispatchEvent(new Event('change',{bubbles:true}));}s.phase='attached';delete s.parts;s.files=s.meta.map(m=>({name:m.name,size:m.size,sha256:m.sha256}));return{ok:true,count:s.meta.length};})()`);
  if (!attached?.ok || attached.count!==files.length) throw error('REFERENCE_UPLOAD_UNCONFIRMED');
  return { count:files.length };
}

export function preSendJournal(saved) {
  return saved?.version==='opencli-task/v1' && ['preparing','submission_unknown','not_submitted'].includes(saved.status)
    && typeof saved.page==='string' && /^[a-zA-Z0-9_-]{4,100}$/.test(saved.page)
    && saved.conversationUrl===null && !Object.hasOwn(saved,'sentAt') && !Object.hasOwn(saved,'sendConfirmed');
}
const emptyCheck = async (page, id) => ({ page:id, ...await page.evaluate(`(()=>{const c=document.querySelector('[contenteditable="true"][role="textbox"],#prompt-textarea');return{url:location.href,userCount:document.querySelectorAll('[data-turn=user],[data-message-author-role=user],.bg-user-message').length,assistantCount:document.querySelectorAll('[data-message-author-role=assistant]').length,composerEmpty:!!c&&!(c.innerText||'').trim(),generating:${activeCode}};})()`), accountKey:(await browserIdentity(page)).accountKey });
function emptyCheckValid(check, saved) {
  return check.page===saved.page && ['https://chatgpt.com/','https://chatgpt.com/new'].includes(check.url)
    && check.accountKey===saved.accountKey && check.userCount===0 && check.assistantCount===0 && check.composerEmpty===true && check.generating===false;
}
function ownedTerminalJournal(saved, proof) {
  if (!preSendJournal(saved) || saved.status!=='not_submitted' || Object.hasOwn(saved,'sendIntent')
      || saved.ownedSend?.version!==OWNED_SEND_PROTOCOL || saved.ownedSend.token!==saved.requestId
      || saved.ownedSend.adapterSha256!==proof.composerAdapterSha256) return false;
  // This exception only reads an already-saved, strictly proven old terminal.
  // ownedPreSendJournal remains current-only, so old unknowns cannot be settled.
  return (proof.runtimeSha256===fs.readFileSync(new URL('./runtime.sha256',import.meta.url),'utf8').trim()
      && proof.composerAdapterSha256===composerAdapterSha256)
    || (proof.runtimeSha256==='f35f97fdbfd3f40d9882b275c8cf13310488a162323f7598ab3557377b039e70'
      && proof.composerAdapterSha256==='0183767038c20ddebf218ddd297b2cf166aceee4d99bb7c03b7f73d7d3654e65')
    || (['54e83e4190239ff9e248082c44008961b0675d9880bb584de026ede24b3c7ec0','afeca4bb4a1f8e59c9850e983882f05593721a2cee019c7a80ca216bf7d817c6','c3d1031294a0a4f1f9a2bf5ca2320797e3e2efaa77c37dcc7c079c05b68c9057','6b85411ff480f21dd3a36a0f26a33f2761a73d591cc810f58ff7beb0710d1aa2'].includes(proof.runtimeSha256)
      && proof.composerAdapterSha256==='5c850d49dba9164a61b499565be9eb21833da724aa6aad6f91a67ddbe86b717b');
}
export function ownedNewPagePreflight(observed, expected) {
  const held=reason=>({ok:false,reason});
  if(expected?.newPageVerified!==true || !expected.page || observed.page!==expected.page
      || (observed.ownerToken!=null&&observed.ownerToken!==expected.requestId))return held('not_owned_new_page');
  if(!['https://chatgpt.com/','https://chatgpt.com/new'].includes(observed.url))return held('not_new_conversation_url');
  if(observed.userCount!==0||observed.assistantCount!==0)return held('existing_conversation');
  if(observed.generating!==false)return held('page_generating');
  if(observed.visibleComposers!==1)return held('composer_not_unique');
  if(observed.embeddedChips!==0)return held('embedded_content_requires_review');
  if(!expected.accountKey||observed.accountKey!==expected.accountKey)return held('account_changed');
  if(observed.logical?.ok!==true||typeof observed.logical.text!=='string')return held('logical_text_unrecognized');
  if(observed.logical.text==='')return{ok:true,mode:'empty'};
  if(typeof expected.prompt==='string'&&expected.prompt&&observed.logical.text===expected.prompt)return{ok:true,mode:'exact_prompt_draft'};
  return held('different_draft');
}
export function ownedNewPagePreflightCode() {
  return `(()=>{const logicalComposerText=${logicalComposerText.toString()};const visible=n=>n instanceof HTMLElement&&n.getClientRects().length>0&&!n.closest('[hidden],[aria-hidden="true"]')&&getComputedStyle(n).display!=='none'&&getComputedStyle(n).visibility!=='hidden';const nodes=[...new Set([...document.querySelectorAll('#prompt-textarea,[data-testid="prompt-textarea"],[contenteditable="true"][role="textbox"]')])].filter(visible);const c=nodes.length===1?nodes[0]:null;return{url:location.href,userCount:document.querySelectorAll('[data-turn=user],[data-message-author-role=user],.bg-user-message').length,assistantCount:document.querySelectorAll('[data-message-author-role=assistant]').length,generating:${activeCode},visibleComposers:nodes.length,embeddedChips:c?c.querySelectorAll('[contenteditable="false"]').length:null,ownerToken:c?c.getAttribute('data-director-owned-composer'):null,logical:c?logicalComposerText(c):{ok:false}};})()`;
}
export async function verifyOwnedNewPagePreflight(page,saved,loc,newPageVerified,identity=browserIdentity) {
  const expected={newPageVerified,page:saved.page,requestId:saved.requestId,accountKey:saved.accountKey,prompt:saved.prompt};
  for(let i=0;i<15;i++) {
    const observed={...await page.evaluate(ownedNewPagePreflightCode()),page:page.getActivePage()};
    if(observed.visibleComposers===1)observed.accountKey=(await identity(page)).accountKey;
    const validation=ownedNewPagePreflight(observed,expected);
    saved.preSendValidation={version:'owned-new-page-preflight/v1',page:observed.page,newPageVerified,url:observed.url,
      userCount:observed.userCount,assistantCount:observed.assistantCount,generating:observed.generating,visibleComposers:observed.visibleComposers,embeddedChips:observed.embeddedChips,
      accountVerified:observed.accountKey===expected.accountKey,logicalRecognized:observed.logical?.ok===true,
      logicalUtf16:typeof observed.logical?.text==='string'?observed.logical.text.length:null,
      logicalSha256:typeof observed.logical?.text==='string'?sha(observed.logical.text):null,expectedPromptSha256:sha(saved.prompt),...validation};
    atomic(loc.file,saved);
    if(validation.ok)return validation;
    if(!newPageVerified||observed.visibleComposers!==0)throw error('EMPTY_PREFLIGHT',true);
    if(i<14)await page.sleep(1);
  }
  throw error('EMPTY_PREFLIGHT',true);
}
function noOwnedProofSendMarkers(value, depth=0) {
  if(depth>32)return false;
  if(Array.isArray(value))return value.every(item=>noOwnedProofSendMarkers(item,depth+1));
  if(value&&typeof value==='object')return Object.entries(value).every(([key,item])=>!['sentAt','sendConfirmed','sendIntent','conversationUrl'].includes(key)&&noOwnedProofSendMarkers(item,depth+1));
  return true;
}
function notSubmittedResult(request,saved,loc) {
  const proof={...saved.notSubmittedProof,journalAfterSha256:sha(fs.readFileSync(loc.file))};
  if (proof.version==='opencli-owned-pre-send-not-submitted/v2') {
    if (!ownedTerminalJournal(saved,proof) || saved.requestId!==request.requestId || requestHash(saved.request)!==saved.requestHash || proof.requestId!==request.requestId || proof.requestHash!==saved.requestHash
        || proof.sendProtocol!==OWNED_SEND_PROTOCOL || proof.clickIntentAbsent!==true || !noOwnedProofSendMarkers(proof)
        || proof.sendMarkerAbsent!==true || proof.sendConfirmedAbsent!==true || proof.conversationUrlAbsent!==true
        || proof.clickCalls!==0 || !/^[a-f0-9]{64}$/.test(proof.runtimeSha256||'') || !/^[a-f0-9]{64}$/.test(proof.journalBeforeSha256||'')
        || proof.journalBeforeSha256===proof.journalAfterSha256 || proof.checks?.length!==2 || !proof.checks.every(c=>ownedCheckValid(c,saved))) throw error('OWNED_NO_CLICK_UNPROVEN');
    return {taskId:request.requestId,status:'failed',resultUrls:[],error:'未提交：提示词或发送按钮准备未通过，已核实本程序没有调用发送；原记录保留。',raw:{transport:'opencli/v1',requestHash:saved.requestHash,opencliNotSubmitted:proof,taskSettlement:{id:request.requestId,status:'failed',reason:'not_submitted'}}};
  }
  if (!preSendJournal(saved) || saved.status!=='not_submitted' || saved.requestId!==request.requestId || requestHash(saved.request)!==saved.requestHash || proof.requestId!==request.requestId || proof.requestHash!==saved.requestHash
      || proof.version!=='opencli-pre-send-not-submitted/v1' || !/^[a-f0-9]{64}$/.test(proof.runtimeSha256||'') || !/^[a-f0-9]{64}$/.test(proof.journalBeforeSha256||'') || proof.journalBeforeSha256===proof.journalAfterSha256
      || proof.sendMarkerAbsent!==true || proof.sendConfirmedAbsent!==true || proof.conversationUrlAbsent!==true || proof.checks?.length!==2 || !proof.checks.every(c=>emptyCheckValid(c,saved))) throw error('PRE_SEND_SETTLEMENT_UNPROVEN');
  return {taskId:request.requestId,status:'failed',resultUrls:[],error:'参考图片准备在发送提示词前中断，已核实未提交生成；可继续制作，原记录保留。',raw:{transport:'opencli/v1',requestHash:saved.requestHash,opencliNotSubmitted:proof,taskSettlement:{id:request.requestId,status:'failed',reason:'not_submitted'}}};
}
export function ownedPreSendJournal(saved) {
  return preSendJournal(saved) && saved.ownedSend?.version===OWNED_SEND_PROTOCOL
    && saved.ownedSend.adapterSha256===composerAdapterSha256 && saved.ownedSend.token===saved.requestId
    && !Object.hasOwn(saved,'sendIntent');
}
const ownedCheckValid=(check,saved)=>check.page===saved.page && ['https://chatgpt.com/','https://chatgpt.com/new'].includes(check.url)
  && check.accountKey===saved.accountKey && check.userCount===0 && check.assistantCount===0 && typeof check.composerEmpty==='boolean' && check.generating===false;
export async function settleOwnedPreSendFailure(config,request,saved,loc,page,check=emptyCheck) {
  if (!ownedPreSendJournal(saved) || !['preparing','submission_unknown'].includes(saved.status)) return null;
  const before=fs.readFileSync(loc.file);checkIdentity(saved,request);
  if(saved.requestId!==request.requestId||requestHash(saved.request)!==saved.requestHash)throw error('REQUEST_IDENTITY_CHANGED');
  const checks=[await check(page,saved.page),await check(page,saved.page)];
  if(!checks.every(c=>ownedCheckValid(c,saved))||!fs.readFileSync(loc.file).equals(before))return null;
  const beforeSha=sha(before),archive=path.join(path.dirname(loc.file),`owned-pre-send-${beforeSha}.json`);
  if(fs.existsSync(archive)){if(!fs.readFileSync(archive).equals(before))throw error('PRE_SEND_ARCHIVE_CHANGED');}else fs.writeFileSync(archive,before,{flag:'wx',mode:0o600});
  saved.status='not_submitted';saved.settledAt=new Date().toISOString();
  saved.notSubmittedProof={version:'opencli-owned-pre-send-not-submitted/v2',sendProtocol:OWNED_SEND_PROTOCOL,composerAdapterSha256,requestId:request.requestId,requestHash:saved.requestHash,runtimeSha256:config.runtimeSha256,journalBeforeSha256:beforeSha,journalBeforeArchive:path.basename(archive),sendMarkerAbsent:true,sendConfirmedAbsent:true,conversationUrlAbsent:true,clickIntentAbsent:true,clickCalls:0,page:saved.page,checks};
  atomic(loc.file,saved);return notSubmittedResult(request,saved,loc);
}
export async function settlePreSendFailure(config,request,saved,loc,page,check=emptyCheck) {
  if (!preSendJournal(saved) || !['preparing','submission_unknown'].includes(saved.status)) return null;
  const before=fs.readFileSync(loc.file);checkIdentity(saved,request);
  if(saved.requestId!==request.requestId || requestHash(saved.request)!==saved.requestHash)throw error('REQUEST_IDENTITY_CHANGED');
  const checks=[await check(page,saved.page),await check(page,saved.page)];
  if (!checks.every(c=>emptyCheckValid(c,saved)) || !fs.readFileSync(loc.file).equals(before)) return null;
  const beforeSha=sha(before),archive=path.join(path.dirname(loc.file),`pre-send-interruption-${beforeSha}.json`);
  if(fs.existsSync(archive)){if(!fs.readFileSync(archive).equals(before))throw error('PRE_SEND_ARCHIVE_CHANGED');}
  else fs.writeFileSync(archive,before,{flag:'wx',mode:0o600});
  saved.status='not_submitted';saved.settledAt=new Date().toISOString();
  saved.notSubmittedProof={version:'opencli-pre-send-not-submitted/v1',requestId:request.requestId,requestHash:saved.requestHash,runtimeSha256:config.runtimeSha256,journalBeforeSha256:beforeSha,journalBeforeArchive:path.basename(archive),sendMarkerAbsent:true,sendConfirmedAbsent:true,conversationUrlAbsent:true,page:saved.page,checks};
  atomic(loc.file,saved);
  return notSubmittedResult(request,saved,loc);
}

async function daemonStatus() {
  let r;
  try { r = await fetch('http://127.0.0.1:19825/status', { headers: { 'X-OpenCLI': '1' }, signal: AbortSignal.timeout(3000) }); }
  catch { return null; }
  if (!r.ok) throw error('LOCAL_SERVICE_UNAVAILABLE');
  return r.json();
}
export async function ensureLocalService(probe = daemonStatus, launch = () => {
  const script = fileURLToPath(new URL('./vendor/opencli/dist/src/daemon.js', import.meta.url));
  const child = spawn(process.execPath, [script], { windowsHide:true, detached:true, stdio:'ignore' });
  child.on('error',()=>{}); child.unref();
}, pause = () => new Promise(resolve=>setTimeout(resolve,250))) {
  if (await probe()) return;
  launch(); // Only start an absent service. Never replace or stop an existing daemon.
  for(let i=0;i<24;i++){await pause();if(await probe())return;}
  throw error('LOCAL_SERVICE_UNAVAILABLE');
}
export function connectedProfile(s, expected = '') {
  const profiles = (s.profiles || []).filter(p => p.extensionConnected);
  const selected = expected ? profiles.filter(p=>p.contextId===expected) : profiles;
  if (selected.length > 1) throw error('BROWSER_SELECTION_REQUIRED');
  if (selected.length !== 1) throw error('OPENCLI_BROWSER_NOT_CONNECTED');
  return selected[0];
}
async function health(profile) {
  const s = await daemonStatus();
  if (!s) throw error('LOCAL_SERVICE_UNAVAILABLE');
  const selected = connectedProfile(s,profile);
  // Global unknown is a cumulative diagnostic counter, not an active request.
  // Frozen task journals and activeOwner retain the actual no-resend gate.
  if (selected.pending) throw error('BRIDGE_BUSY');
  return selected.contextId;
}
// Read only bounded, direct UI text outside navigation, conversations and drafts.
// A normal missing composer/hydration is deliberately not an account failure.
export function accountLoadFailure() {
  if (document.querySelector('[contenteditable="true"][role="textbox"],#prompt-textarea')) return false;
  const excluded='nav,aside,[role="navigation"],[data-testid*="sidebar"],[id*="sidebar"],#history,[data-message-author-role],[data-turn],article,pre,code,.markdown,[contenteditable],textarea';
  const titles=new Set(['无法加载你的账户','无法加载您的账户','Unable to load your account',"We couldn't load your account"]);
  let visited=0;
  for (const element of document.querySelectorAll('h1,h2,h3,[role="heading"],[role="alert"],p,div,span')) {
    if (++visited>1200) break;
    if (element.closest(excluded) || element.closest('[hidden],[aria-hidden="true"]') || !element.getClientRects().length) continue;
    const style=getComputedStyle(element);if(style.display==='none'||style.visibility==='hidden'||style.visibility==='collapse')continue;
    let text='';
    for(const node of element.childNodes) {
      if(node.nodeType!==3)continue;
      if(typeof node.data!=='string'||node.data.length>256){text='';break;}
      text+=node.data;if(text.length>256){text='';break;}
    }
    if(titles.has(text.trim().replace(/[.!。！]$/u,'')))return true;
  }
  return false;
}
export async function browserIdentity(p) {
  try {return await p.evaluate(`(async()=>{if(location.protocol==='chrome-error:')throw Error('BROWSER_NETWORK_UNAVAILABLE');if(location.origin!=='https://chatgpt.com')throw Error('CHATGPT_PAGE_LOADING');if((${accountLoadFailure.toString()})())throw Error('CHATGPT_ACCOUNT_LOAD_FAILED');let r;try{r=await fetch('/api/auth/session',{credentials:'include',signal:AbortSignal.timeout(6000)});}catch{throw Error('BROWSER_NETWORK_UNAVAILABLE');}if(r.status===401)throw Error('AUTH_REQUIRED');if(r.status===403)throw Error('BROWSER_CHECK_REQUIRED');if(r.status===429)throw Error('BRIDGE_BUSY');if(!r.ok)throw Error('BROWSER_NETWORK_UNAVAILABLE');const s=await r.json();const id=s.user?.id;if(typeof id!=='string'||!id)throw Error('AUTH_REQUIRED');const key=[...new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(id)))].map(b=>b.toString(16).padStart(2,'0')).join('');return{accountKey:key,browserKind:/Edg\\//.test(navigator.userAgent)?'edge':/Chrome\\//.test(navigator.userAgent)?'chrome':'other',composer:!!document.querySelector('[contenteditable="true"][role="textbox"],#prompt-textarea')};})()`);}catch(e){throw identityError(e);}
}
export function identityError(e) {
  for(const code of ['BROWSER_NETWORK_UNAVAILABLE','BROWSER_CHECK_REQUIRED','CHATGPT_PAGE_LOADING','CHATGPT_ACCOUNT_LOAD_FAILED','BRIDGE_BUSY','AUTH_REQUIRED']) {
    if(String(e.message).includes(code)) return error(code);
  }
  return e;
}
export async function waitForIdentity(p, expectedAccount = '', attempts = 15) {
  let identity, loading=false;
  for(let i=0;i<attempts;i++) {
    try { identity=await browserIdentity(p);loading=false; }
    catch(e) {const classified=identityError(e);if(classified.code==='CHATGPT_PAGE_LOADING'){loading=true;await p.sleep(1);continue;}if(!String(e.message).includes('AUTH_REQUIRED'))throw classified;identity=null;loading=false;}
    if(identity?.accountKey && expectedAccount && identity.accountKey!==expectedAccount)throw error('ACCOUNT_CHANGED');
    if(identity?.composer)return identity;
    await p.sleep(1);
  }
  if(loading)throw error('CHATGPT_PAGE_LOADING');
  if(identity?.accountKey)throw error('BRIDGE_BUSY'); // Authenticated page still hydrating, not logout.
  throw error('AUTH_REQUIRED');
}
async function inspect(config, connect = false) {
  if (connect) await ensureLocalService();
  const profile = await health(config.profile), p = session(profile), tabs = await p.tabs();
  let tab = tabs.find(t => t.page && /^https:\/\/chatgpt.com(?:\/|$)/.test(t.url || ''));
  if (!tab && connect) { const page = await p.newTab('https://chatgpt.com/'); tab = { page }; await p.sleep(2); }
  if (!tab) throw error('OPENCLI_OWNED_PAGE_REQUIRED');
  p.setActivePage(tab.page);
  const expectedBrowser=config.requestedBrowser||config.browserKind;
  if(expectedBrowser) {
    const browser=await p.evaluate(`(/Edg\\//.test(navigator.userAgent)?'edge':/Chrome\\//.test(navigator.userAgent)?'chrome':'other')`);
    if(browser!==expectedBrowser)throw error('BROWSER_KIND_MISMATCH');
  }
  const identity = await waitForIdentity(p,config.accountKey);
  if (config.accountKey && identity.accountKey !== config.accountKey) throw error('ACCOUNT_CHANGED');
  return { profile, ...identity, page: tab.page, transport: 'opencli/v1', ready: true };
}
export async function connectAvailable(config, probe = daemonStatus, inspectProfile = inspect) {
  const status = await probe(); if(!status)throw error('LOCAL_SERVICE_UNAVAILABLE');
  const profiles=(status.profiles||[]).filter(p=>p.extensionConnected);
  if(config.requestedBrowser && !['chrome','edge'].includes(config.requestedBrowser))throw error('INVALID_CONFIG');
  if(config.allowAccountChange && !config.requestedBrowser)throw error('INVALID_CONFIG');
  if(!config.accountKey && !config.requestedBrowser && profiles.length>1)throw error('BROWSER_SELECTION_REQUIRED');
  const ordered=[...profiles].sort((a,b)=>Number(b.contextId===config.profile)-Number(a.contextId===config.profile));
  let last=error('OPENCLI_BROWSER_NOT_CONNECTED');
  const candidates=[];
  for(const profile of ordered.slice(0,4)) {
    try {
      const ready=await inspectProfile({...config,profile:profile.contextId,accountKey:config.allowAccountChange?'':config.accountKey},true);
      if(config.requestedBrowser && ready.browserKind!==config.requestedBrowser)throw error('BROWSER_KIND_MISMATCH');
      if(config.accountKey && ready.accountKey!==config.accountKey && !config.allowAccountChange)throw error('ACCOUNT_CHANGED');
      if(!config.requestedBrowser || profile.contextId===config.profile)return ready;
      candidates.push(ready);
    } catch(e) {if(e.code!=='BROWSER_KIND_MISMATCH')last=e;}
  }
  if(candidates.length>1)throw error('BROWSER_SELECTION_REQUIRED');
  if(candidates.length===1)return candidates[0];
  throw last;
}
function paths(config, id) {
  if (!path.isAbsolute(config.stateRoot) || !/^[a-zA-Z0-9_-]{1,100}$/.test(id)) throw error('INVALID_STATE_PATH');
  fs.mkdirSync(config.stateRoot, { recursive: true });
  if (fs.lstatSync(config.stateRoot).isSymbolicLink()) throw error('INVALID_STATE_PATH');
  const root = fs.realpathSync(config.stateRoot), dir = path.join(root, id);
  fs.mkdirSync(dir, { recursive: true }); if (fs.lstatSync(dir).isSymbolicLink()) throw error('INVALID_STATE_PATH');
  return { root, dir, file: path.join(dir, 'request.json'), active: path.join(root, 'active.json') };
}
function read(file) { return JSON.parse(fs.readFileSync(file, 'utf8')); }
export function activeOwner(loc, requestId) {
  if (!fs.existsSync(loc.active)) return;
  const active = read(loc.active);
  if (active.requestId === requestId) return;
  const previous = path.join(loc.root, active.requestId, 'request.json');
  if (!fs.existsSync(previous)) throw error('BRIDGE_BUSY');
  const saved=read(previous);
  if (saved.status!=='downloaded') {
    if (saved.status!=='not_submitted' || !preSendJournal(saved) || saved.notSubmittedProof?.requestId!==active.requestId) throw error('BRIDGE_BUSY');
    notSubmittedResult(saved.request,saved,{file:previous});
  }
}
async function submit(config, request) {
  validateRequest(request); const loc = paths(config, request.requestId), hash = requestHash(request);
  // Exclusive journal is the durable at-most-once send gate, even after process death.
  if (fs.existsSync(loc.file)) { const previous = read(loc.file); checkIdentity(previous, request); return { taskId: request.requestId, status: previous.status === 'downloaded' ? 'succeeded' : 'processing', resultUrls: previous.resultUrls || [], raw: { transport: 'opencli/v1', recoveredReceipt: true } }; }
  const ready = await inspect(config,true); activeOwner(loc, request.requestId);
  // Rust holds the shared OS command lease; it releases automatically after a crash.
  // Only this brief local lock is removed; active.json deliberately survives unknown work.
  let saved = { version: 'opencli-task/v1', requestId: request.requestId, requestHash: hash, request, profile: ready.profile, accountKey: ready.accountKey, prompt: webImagePrompt(request), status: 'preparing', createdAt: new Date().toISOString(), page: null, conversationUrl: null, ownedSend:{version:OWNED_SEND_PROTOCOL,adapterSha256:composerAdapterSha256,token:request.requestId} };
  let p;
  try {
    activeOwner(loc, request.requestId);
    fs.writeFileSync(loc.file, JSON.stringify(saved), { flag: 'wx', mode: 0o600 }); atomic(loc.active, { requestId: request.requestId, profile: ready.profile });
    p = session(ready.profile); const priorPages=new Set((await p.tabs()).map(tab=>tab.page));
    saved.page = await p.newTab('https://chatgpt.com/new'); atomic(loc.file, saved); if (!saved.page) throw error('TAB_OPEN_UNKNOWN', true);
    p.setActivePage(saved.page);await verifyOwnedNewPagePreflight(p,saved,loc,!priorPages.has(saved.page));
    if (request.references.length) { const refs = [...request.references].sort((a,b) => a.order - b.order); const upload = await uploadChatGPTImages({setFileInput:()=>attachReferenceFiles(p,refs,request.requestId),evaluate:p.evaluate.bind(p),sleep:p.sleep.bind(p)},refs.map(r => r.path)); if (!upload?.ok) throw error('REFERENCE_UPLOAD_UNCONFIRMED', true); }
    await submitOwnedComposer(p,saved,loc);
    atomic(loc.file, saved); return { taskId: request.requestId, status: 'processing', resultUrls: [], raw: { transport: 'opencli/v1', requestHash: hash } };
  } catch (e) { if (fs.existsSync(loc.file)) { saved.status = 'submission_unknown'; saved.error = e.code || 'OPENCLI_INTERRUPTED'; atomic(loc.file, saved);
      if(p&&ownedPreSendJournal(saved)){try{const settled=await settleOwnedPreSendFailure(config,request,saved,loc,p);if(settled)return settled;}catch{/* No proof means the original task stays held. */}}
    } throw error(e.code || 'SUBMISSION_UNKNOWN', true); }
  finally { /* Durable request and active owner remain for original-result recovery. */ }
}
export async function submitOwnedComposer(page,saved,loc) {
  const gate=await prepareOwnedComposer(page,saved.prompt,saved.requestId);
  saved.status='send_dispatching';saved.sentAt=new Date().toISOString();
  saved.sendIntent={version:'owned-click-intent/v1',token:gate.token,promptSha256:gate.promptSha256,requestId:saved.requestId,requestHash:saved.requestHash,page:saved.page};
  atomic(loc.file,saved);
  await clickOwnedComposer(page,saved.prompt,gate,saved.sendIntent);
  const observed=await observeOwnedSubmission(page,saved.prompt);
  saved.status='processing';saved.sendConfirmed=true;saved.conversationUrl=observed.conversationUrl;
}
export function recordPassiveObservation(config,saved,loc,state,journalBefore) {
  if(state.userCount!==1||state.promptMatched!==true||!stableConversation(state.url)
      ||(saved.conversationUrl&&saved.conversationUrl!==state.url))throw error('ORIGINAL_PROMPT_UNCONFIRMED');
  if(saved.ownedSend) {
    const intent=saved.sendIntent;
    if(saved.ownedSend.version!==OWNED_SEND_PROTOCOL||saved.ownedSend.token!==saved.requestId
        ||!Number.isFinite(Date.parse(saved.sentAt))||intent?.version!=='owned-click-intent/v1'
        ||intent.token!==saved.requestId||intent.requestId!==saved.requestId||intent.requestHash!==saved.requestHash
        ||intent.promptSha256!==sha(saved.prompt)||typeof intent.page!=='string'||!intent.page
        ||(intent.page!==saved.page&&!stableConversation(saved.conversationUrl)))throw error('ORIGINAL_SEND_INTENT_UNCONFIRMED');
  }
  if(!fs.readFileSync(loc.file).equals(journalBefore))throw error('ORIGINAL_JOURNAL_CHANGED');
  if(!saved.sendConfirmed) {
    const beforeSha=sha(journalBefore),name=`request.before-passive-observation.${beforeSha}.json`,archive=path.join(path.dirname(loc.file),name);
    if(fs.existsSync(archive)){if(!fs.readFileSync(archive).equals(journalBefore))throw error('ORIGINAL_ARCHIVE_CHANGED');}
    else fs.writeFileSync(archive,journalBefore,{flag:'wx',mode:0o600});
    saved.passiveObservation={version:'opencli-passive-observation/v1',requestId:saved.requestId,requestHash:saved.requestHash,
      promptSha256:sha(saved.prompt),profile:saved.profile,accountKey:saved.accountKey,page:saved.page,conversationUrl:state.url,
      runtimeSha256:config.runtimeSha256,journalBeforeSha256:beforeSha,journalBeforeArchive:name,readOnly:true,resubmitted:false,
      observedAt:new Date().toISOString()};
  }
  saved.sendConfirmed=true;saved.conversationUrl=state.url;
  // Keep the original error/status as recovery evidence until original bytes
  // are fully verified. Observing the DOM does not issue or retry a generation.
  atomic(loc.file,saved);
}
// Runs only inside the bound page. Credentials stay inside the browser and only
// a verified original image is returned. No conversation API or send endpoint.
export async function readOwnedOriginalImage(expected,conversationUrl,expectedAccount) {
  const inspect=()=>{
    const state=readOwnedConversationDom(expected);
    if(generationActive())throw Error('ORIGINAL_GENERATION_ACTIVE');
    if(state.url!==conversationUrl||!/^https:\/\/chatgpt\.com\/c\/[a-z0-9-]+$/.test(state.url)||state.userCount!==1||!state.promptMatched)throw Error('ORIGINAL_PROMPT_UNCONFIRMED');
    if(state.imageCandidates!==1||state.images.length!==1||state.selectors>1)throw Error('ORIGINAL_IMAGE_UNCONFIRMED');
    return state.images[0];
  };
  const img=inspect(),source=img.currentSrc||img.src;
  const fetchRead=(url,options={})=>fetch(url,{...options,signal:AbortSignal.timeout(8000)});
  const hash=async b=>[...new Uint8Array(await crypto.subtle.digest('SHA-256',b))].map(b=>b.toString(16).padStart(2,'0')).join('');
  const previewReply=await fetchRead(source);if(!previewReply.ok)throw Error('ORIGINAL_PREVIEW_UNAVAILABLE');
  const preview=await previewReply.arrayBuffer(),wanted=await hash(preview);
  const urls=[...new Map(performance.getEntriesByType('resource').map(e=>e.name).filter(u=>{
    try{const parsed=new URL(u);return parsed.origin===location.origin&&/^\/backend-api\/files\/download\/file_[a-z0-9]+$/.test(parsed.pathname);}catch{return false;}
  }).map(u=>[new URL(u).pathname,u])).values()];
  if(!urls.length||urls.length>12)throw Error('ORIGINAL_FILE_IDENTITY');
  const auth=await fetchRead('/api/auth/session',{credentials:'include'});if(!auth.ok)throw Error('AUTH_REQUIRED');
  const session=await auth.json();if(!session.accessToken||typeof session.user?.id!=='string'||!session.user.id)throw Error('AUTH_REQUIRED');
  if(!/^[a-f0-9]{64}$/.test(expectedAccount||'')||await hash(new TextEncoder().encode(session.user.id))!==expectedAccount)throw Error('ACCOUNT_CHANGED');
  const matches=await Promise.all(urls.map(async u=>{
    const m=await fetchRead(u,{credentials:'include',headers:{Authorization:'Bearer '+session.accessToken}});if(!m.ok)return null;
    const v=await m.json();if(typeof v.download_url!=='string')return null;
    const x=new URL(v.download_url);
    if(x.protocol!=='https:'||!['chatgpt.com','files.oaiusercontent.com'].some(h=>x.hostname===h||x.hostname.endsWith('.'+h)))throw Error('ORIGINAL_FILE_HOST');
    const r=await fetchRead(x.href,{credentials:'include'});if(!r.ok)return null;
    const blob=await r.blob(),b=await blob.arrayBuffer(),digest=await hash(b);
    if(digest!==wanted||b.byteLength!==preview.byteLength)return null;
    const dataUrl=await new Promise((resolve,reject)=>{const reader=new FileReader();reader.onload=()=>resolve(reader.result);reader.onerror=reject;reader.readAsDataURL(blob);});
    return{sha256:digest,bytes:b.byteLength,mime:blob.type,fileId:new URL(u).pathname.split('/').pop(),dataUrl};
  }));
  const match=matches.filter(Boolean);if(match.length!==1)throw Error('ORIGINAL_FILE_AMBIGUOUS');
  const finalImage=inspect();if((finalImage.currentSrc||finalImage.src)!==source)throw Error('ORIGINAL_IMAGE_CHANGED');
  return match[0];
}
export function originalImageReadCode(prompt,conversationUrl,expectedAccount) {
  return `(async()=>{const readOwnedConversationDom=${readOwnedConversationDom.toString()};const generationActive=()=>(${activeCode});const readOwnedOriginalImage=${readOwnedOriginalImage.toString()};return readOwnedOriginalImage(${JSON.stringify(prompt)},${JSON.stringify(conversationUrl)},${JSON.stringify(expectedAccount)});})()`;
}
async function query(config, request) {
  validateRequest(request,false); const loc = paths(config, request.requestId);
  if (!fs.existsSync(loc.file)) throw error('ORIGINAL_TASK_MISSING'); const journalBefore=fs.readFileSync(loc.file),saved=JSON.parse(journalBefore); checkIdentity(saved, request);
  if (saved.profile !== config.profile || saved.accountKey !== config.accountKey) throw error('ACCOUNT_CHANGED');
  if (saved.status === 'downloaded') return { taskId: request.requestId, status: 'succeeded', resultUrls: saved.resultUrls, raw: saved.proof };
  if (saved.status === 'not_submitted') return notSubmittedResult(request,saved,loc);
  await health(config.profile); const p = session(saved.profile); p.setActivePage(saved.page);
  if (ownedPreSendJournal(saved)) { const settled=await settleOwnedPreSendFailure(config,request,saved,loc,p);if(settled)return settled; }
  else if (preSendJournal(saved)) { const settled=await settlePreSendFailure(config,request,saved,loc,p);if(settled)return settled; }
  let url;
  try { url = await p.evaluate('location.href'); } catch { if (!stableConversation(saved.conversationUrl)) throw error('ORIGINAL_PAGE_UNAVAILABLE'); const page = await p.newTab(saved.conversationUrl); saved.page = page; p.setActivePage(page); await p.sleep(1); url = await p.evaluate('location.href'); }
  if (stableConversation(url)) { if (saved.conversationUrl && saved.conversationUrl !== url) throw error('CONVERSATION_CHANGED'); }
  else if (saved.conversationUrl) throw error('CONVERSATION_CHANGED');
  const identity = await browserIdentity(p); if (identity.accountKey !== saved.accountKey) throw error('ACCOUNT_CHANGED');
  const state = await p.evaluate(ownedConversationStateCode(saved.prompt,activeCode));
  if (state.userCount===0 && Date.now()-Date.parse(saved.sentAt||saved.createdAt)<60000) return {taskId:request.requestId,status:'processing',resultUrls:[],raw:{transport:'opencli/v1',phase:'loading_original_page'}};
  if (state.userCount!==1 || state.promptMatched!==true) throw error('ORIGINAL_PROMPT_UNCONFIRMED');
  if (state.url!==url || !stableConversation(state.url)) throw error('CONVERSATION_CHANGED');
  recordPassiveObservation(config,saved,loc,state,journalBefore);
  if(Date.now()-Date.parse(saved.sentAt||saved.createdAt)>15*60*1000&&state.count===0)throw error('ORIGINAL_RESULT_TIMEOUT');
  if (state.imageCandidates>1 || state.selectors>1) throw error('MULTIPLE_IMAGES_REQUIRE_REVIEW');
  if (state.generating || state.count===0) return { taskId:request.requestId,status:'processing',resultUrls:[],raw:{transport:'opencli/v1',conversationUrl:saved.conversationUrl,observedImages:state.count} };
  // Re-read the exact conversation/prompt and its unique generated image in the
  // same evaluation as original-byte matching; never download an arbitrary blob.
  const asset=await p.evaluate(originalImageReadCode(saved.prompt,saved.conversationUrl,saved.accountKey));
  const bytes = Buffer.from(asset.dataUrl.replace(/^data:[^;]+;base64,/,''), 'base64');
  if (asset.mime !== 'image/png' || sha(bytes) !== asset.sha256 || bytes.length !== asset.bytes || bytes.subarray(0,8).toString('hex') !== '89504e470d0a1a0a') throw error('ORIGINAL_BYTES_INVALID');
  const file = path.join(loc.dir, asset.sha256 + '.png'); if (!fs.existsSync(file)) fs.writeFileSync(file, bytes, { flag:'wx', mode:0o600 });
  saved.status = 'downloaded'; saved.resultUrls = [`opencli-image:${request.requestId}:${asset.sha256}`]; saved.proof = { transport:'opencli/v1', requestHash:saved.requestHash, promptSha256:sha(saved.prompt), conversationUrl:saved.conversationUrl, sha256:asset.sha256, bytes:asset.bytes, mime:asset.mime, originalIdentityVerified:true, fileId:asset.fileId, path:file }; saved.finishedAt = new Date().toISOString(); atomic(loc.file,saved);
  return { taskId:request.requestId, status:'succeeded', resultUrls:saved.resultUrls, raw:saved.proof };
}
export async function dispatch(input) {
  const { operation, config, request } = input;
  if (!config || !path.isAbsolute(config.stateRoot) || (config.profile && !/^[a-zA-Z0-9_-]{4,100}$/.test(config.profile))) throw error('INVALID_CONFIG');
  if (operation === 'status' || operation === 'connect') setDaemonCommandTimeoutSeconds(8);
  if (operation === 'status') return inspect(config,false);
  if (operation === 'connect') {await ensureLocalService();return connectAvailable(config);}
  if (operation === 'prepare') { validateRequest(request); const ready=await inspect(config,true); const loc=paths(config,request.requestId); activeOwner(loc,request.requestId); return { ...ready, requestHash:requestHash(request), sendProtocol:OWNED_SEND_PROTOCOL, composerAdapterSha256 }; }
  if (operation === 'submit') return submit(config,request);
  if (operation === 'query') return query(config,request);
  if (operation === 'cleanup') {
    validateRequest(request,false);const loc=paths(config,request.requestId),saved=read(loc.file);checkIdentity(saved,request);
    if(saved.status!=='downloaded'||!saved.proof?.originalIdentityVerified||saved.profile!==config.profile||saved.accountKey!==config.accountKey)throw error('CLEANUP_IDENTITY');
    if(saved.pageClosed)return{status:'already_closed'};
    const p=session(saved.profile);p.setActivePage(saved.page);const tabs=await p.tabs();
    if(!tabs.some(t=>t.page===saved.page))return{status:'retained',reason:'page_absence_unproven'};
    if(await p.evaluate(activeCode)||await p.evaluate('location.href')!==saved.conversationUrl)return{status:'retained',reason:'ownership_changed'};
    // Closing OpenCLI's preferred tab releases the whole lease. Select an owned
    // controller first, so cleanup removes only this completed task page.
    let controller=tabs.find(t=>t.page!==saved.page&&/^https:\/\/chatgpt.com(?:\/|$)/.test(t.url||''))?.page;
    if(!controller)controller=await p.newTab('https://chatgpt.com/');
    if(!controller||controller===saved.page)throw error('CONTROL_PAGE_UNCONFIRMED');
    await p.selectTab(controller);
    await p.closeTab(saved.page);saved.pageClosed=true;atomic(loc.file,saved);return{status:'closed'};
  }
  throw error('UNSUPPORTED_OPERATION');
}
const nativePath = p => path.resolve(p).replace(/^\\\\\?\\/, '').toLowerCase();
if (process.argv[1] && nativePath(fileURLToPath(import.meta.url)) === nativePath(process.argv[1])) {
  try { let text=''; for await (const chunk of process.stdin) {text+=chunk;if(text.length>500000)throw error('INPUT_TOO_LARGE');} const data=await dispatch(JSON.parse(text)); process.stdout.write(JSON.stringify({ok:true,data})); }
  catch(e) { process.stdout.write(JSON.stringify({ok:false,error:{code:e.code||'OPENCLI_BRIDGE_ERROR',submissionUnknown:!!e.unknown,diagnostic:{name:e.name,message:String(e.message).slice(0,500)}}})); process.exitCode=1; }
}
