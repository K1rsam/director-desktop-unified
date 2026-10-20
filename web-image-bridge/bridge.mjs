// ChatGPT web transport only. No image API, cookie export, video or automatic resend.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { Page } from './vendor/opencli/dist/src/browser/page.js';
import { setDaemonCommandTimeoutSeconds } from './vendor/opencli/dist/src/browser/daemon-client.js';
import { sendChatGPTMessage, uploadChatGPTImages } from './vendor/opencli/clis/chatgpt/utils.js';

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
function notSubmittedResult(request,saved,loc) {
  const proof={...saved.notSubmittedProof,journalAfterSha256:sha(fs.readFileSync(loc.file))};
  if (!preSendJournal(saved) || saved.status!=='not_submitted' || saved.requestId!==request.requestId || requestHash(saved.request)!==saved.requestHash || proof.requestId!==request.requestId || proof.requestHash!==saved.requestHash
      || proof.version!=='opencli-pre-send-not-submitted/v1' || !/^[a-f0-9]{64}$/.test(proof.runtimeSha256||'') || !/^[a-f0-9]{64}$/.test(proof.journalBeforeSha256||'') || proof.journalBeforeSha256===proof.journalAfterSha256
      || proof.sendMarkerAbsent!==true || proof.sendConfirmedAbsent!==true || proof.conversationUrlAbsent!==true || proof.checks?.length!==2 || !proof.checks.every(c=>emptyCheckValid(c,saved))) throw error('PRE_SEND_SETTLEMENT_UNPROVEN');
  return {taskId:request.requestId,status:'failed',resultUrls:[],error:'参考图片准备在发送提示词前中断，已核实未提交生成；可继续制作，原记录保留。',raw:{transport:'opencli/v1',requestHash:saved.requestHash,opencliNotSubmitted:proof,taskSettlement:{id:request.requestId,status:'failed',reason:'not_submitted'}}};
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
export async function browserIdentity(p) {
  try {return await p.evaluate(`(async()=>{if(location.protocol==='chrome-error:')throw Error('BROWSER_NETWORK_UNAVAILABLE');if(location.origin!=='https://chatgpt.com')throw Error('CHATGPT_PAGE_LOADING');let r;try{r=await fetch('/api/auth/session',{credentials:'include',signal:AbortSignal.timeout(6000)});}catch{throw Error('BROWSER_NETWORK_UNAVAILABLE');}if(r.status===401)throw Error('AUTH_REQUIRED');if(r.status===403)throw Error('BROWSER_CHECK_REQUIRED');if(r.status===429)throw Error('BRIDGE_BUSY');if(!r.ok)throw Error('BROWSER_NETWORK_UNAVAILABLE');const s=await r.json();const id=s.user?.id;if(typeof id!=='string'||!id)throw Error('AUTH_REQUIRED');const key=[...new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(id)))].map(b=>b.toString(16).padStart(2,'0')).join('');return{accountKey:key,browserKind:/Edg\\//.test(navigator.userAgent)?'edge':/Chrome\\//.test(navigator.userAgent)?'chrome':'other',composer:!!document.querySelector('[contenteditable="true"][role="textbox"],#prompt-textarea')};})()`);}catch(e){throw identityError(e);}
}
export function identityError(e) {
  for(const code of ['BROWSER_NETWORK_UNAVAILABLE','BROWSER_CHECK_REQUIRED','CHATGPT_PAGE_LOADING','BRIDGE_BUSY','AUTH_REQUIRED']) {
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
function activeOwner(loc, requestId) {
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
  let saved = { version: 'opencli-task/v1', requestId: request.requestId, requestHash: hash, request, profile: ready.profile, accountKey: ready.accountKey, prompt: webImagePrompt(request), status: 'preparing', createdAt: new Date().toISOString(), page: null, conversationUrl: null };
  try {
    activeOwner(loc, request.requestId);
    fs.writeFileSync(loc.file, JSON.stringify(saved), { flag: 'wx', mode: 0o600 }); atomic(loc.active, { requestId: request.requestId, profile: ready.profile });
    const p = session(ready.profile); saved.page = await p.newTab('https://chatgpt.com/new'); atomic(loc.file, saved); if (!saved.page) throw error('TAB_OPEN_UNKNOWN', true);
    p.setActivePage(saved.page); let blank = false;
    for (let i = 0; i < 15; i++) { blank = await p.evaluate(`(()=>{const c=document.querySelector('[contenteditable="true"][role="textbox"],#prompt-textarea');return !!c&&!(c.innerText||'').trim()&&document.querySelectorAll('[data-turn=user],[data-message-author-role=user],.bg-user-message').length===0;})()`); if (blank) break; await p.sleep(1); }
    if (!blank || await p.evaluate(activeCode)) throw error('EMPTY_PREFLIGHT', true);
    if (request.references.length) { const refs = [...request.references].sort((a,b) => a.order - b.order); const upload = await uploadChatGPTImages({setFileInput:()=>attachReferenceFiles(p,refs,request.requestId),evaluate:p.evaluate.bind(p),sleep:p.sleep.bind(p)},refs.map(r => r.path)); if (!upload?.ok) throw error('REFERENCE_UPLOAD_UNCONFIRMED', true); }
    saved.status = 'send_dispatching'; saved.sentAt = new Date().toISOString(); atomic(loc.file, saved);
    const sent = await sendChatGPTMessage(p, saved.prompt); if (!sent) throw error('SUBMISSION_UNKNOWN', true);
    saved.status = 'processing'; saved.sendConfirmed = true;
    const url = await p.evaluate('location.href'); if (stableConversation(url)) saved.conversationUrl = url;
    atomic(loc.file, saved); return { taskId: request.requestId, status: 'processing', resultUrls: [], raw: { transport: 'opencli/v1', requestHash: hash } };
  } catch (e) { if (fs.existsSync(loc.file)) { saved.status = 'submission_unknown'; saved.error = e.code || 'OPENCLI_INTERRUPTED'; atomic(loc.file, saved); } throw error(e.code || 'SUBMISSION_UNKNOWN', true); }
  finally { /* Durable request and active owner remain for original-result recovery. */ }
}
async function query(config, request) {
  validateRequest(request,false); const loc = paths(config, request.requestId);
  if (!fs.existsSync(loc.file)) throw error('ORIGINAL_TASK_MISSING'); const saved = read(loc.file); checkIdentity(saved, request);
  if (saved.profile !== config.profile || saved.accountKey !== config.accountKey) throw error('ACCOUNT_CHANGED');
  if (saved.status === 'downloaded') return { taskId: request.requestId, status: 'succeeded', resultUrls: saved.resultUrls, raw: saved.proof };
  if (saved.status === 'not_submitted') return notSubmittedResult(request,saved,loc);
  await health(config.profile); const p = session(saved.profile); p.setActivePage(saved.page);
  if (preSendJournal(saved)) { const settled=await settlePreSendFailure(config,request,saved,loc,p);if(settled)return settled; }
  let url;
  try { url = await p.evaluate('location.href'); } catch { if (!stableConversation(saved.conversationUrl)) throw error('ORIGINAL_PAGE_UNAVAILABLE'); const page = await p.newTab(saved.conversationUrl); saved.page = page; p.setActivePage(page); atomic(loc.file, saved); await p.sleep(1); url = await p.evaluate('location.href'); }
  if (stableConversation(url)) { if (saved.conversationUrl && saved.conversationUrl !== url) throw error('CONVERSATION_CHANGED'); saved.conversationUrl = url; atomic(loc.file, saved); }
  else if (saved.conversationUrl) throw error('CONVERSATION_CHANGED');
  const identity = await browserIdentity(p); if (identity.accountKey !== saved.accountKey) throw error('ACCOUNT_CHANGED');
  const state = await p.evaluate(`(()=>{const users=[...document.querySelectorAll('[data-turn=user],[data-message-author-role=user],.bg-user-message')];const images=[...document.querySelectorAll('[data-testid="generated-image-gallery"] img')].filter(i=>i.complete&&i.naturalWidth>128);const selectors=[...document.querySelectorAll('button[aria-label]')].map(b=>b.getAttribute('aria-label')).filter(l=>/^(显示生成的图像|Show generated image) [0-9]+$/.test(l));return{url:location.href,userTexts:users.map(n=>n.textContent),count:images.length,selectors:selectors.length,generating:${activeCode}};})()`);
  if (!state.userTexts.length && Date.now()-Date.parse(saved.sentAt||saved.createdAt)<60000) return {taskId:request.requestId,status:'processing',resultUrls:[],raw:{transport:'opencli/v1',phase:'loading_original_page'}};
  if (state.userTexts.length !== 1 || !state.userTexts[0].includes(saved.prompt)) throw error('ORIGINAL_PROMPT_UNCONFIRMED');
  if(Date.now()-Date.parse(saved.sentAt||saved.createdAt)>15*60*1000&&state.count===0)throw error('ORIGINAL_RESULT_TIMEOUT');
  if (state.generating || state.count === 0 || !stableConversation(url)) return { taskId: request.requestId, status: 'processing', resultUrls: [], raw: { transport:'opencli/v1', conversationUrl:saved.conversationUrl, observedImages:state.count } };
  // One frozen asset per request avoids assuming that multi-image output order equals asset order.
  if (state.count !== 1 || state.selectors > 1) throw error('MULTIPLE_IMAGES_REQUIRE_REVIEW');
  const asset = await p.evaluate(`(async()=>{const img=[...document.querySelectorAll('[data-testid="generated-image-gallery"] img')].find(i=>i.complete&&i.naturalWidth>128);const hash=async b=>[...new Uint8Array(await crypto.subtle.digest('SHA-256',b))].map(b=>b.toString(16).padStart(2,'0')).join('');const preview=await(await fetch(img.currentSrc||img.src)).arrayBuffer(),wanted=await hash(preview);const urls=[...new Map(performance.getEntriesByType('resource').map(e=>e.name).filter(u=>new URL(u).origin===location.origin&&/^\\/backend-api\\/files\\/download\\/file_[a-z0-9]+$/.test(new URL(u).pathname)).map(u=>[new URL(u).pathname,u])).values()];if(!urls.length||urls.length>12)throw Error('ORIGINAL_FILE_IDENTITY');const session=await(await fetch('/api/auth/session',{credentials:'include'})).json();if(!session.accessToken)throw Error('AUTH_REQUIRED');const matches=await Promise.all(urls.map(async u=>{const m=await fetch(u,{credentials:'include',headers:{Authorization:'Bearer '+session.accessToken}});if(!m.ok)return null;const v=await m.json();if(typeof v.download_url!=='string')return null;const x=new URL(v.download_url);if(x.protocol!=='https:'||!['chatgpt.com','files.oaiusercontent.com'].some(h=>x.hostname===h||x.hostname.endsWith('.'+h)))throw Error('ORIGINAL_FILE_HOST');const r=await fetch(x.href,{credentials:'include'});if(!r.ok)return null;const blob=await r.blob(),b=await blob.arrayBuffer(),digest=await hash(b);if(digest!==wanted||b.byteLength!==preview.byteLength)return null;const dataUrl=await new Promise((resolve,reject)=>{const reader=new FileReader();reader.onload=()=>resolve(reader.result);reader.onerror=reject;reader.readAsDataURL(blob);});return{sha256:digest,bytes:b.byteLength,mime:blob.type,fileId:new URL(u).pathname.split('/').pop(),dataUrl};}));const match=matches.filter(Boolean);if(match.length!==1)throw Error('ORIGINAL_FILE_AMBIGUOUS');return match[0];})()`);
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
  if (operation === 'prepare') { validateRequest(request); const ready=await inspect(config,true); const loc=paths(config,request.requestId); activeOwner(loc,request.requestId); return { ...ready, requestHash:requestHash(request) }; }
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
