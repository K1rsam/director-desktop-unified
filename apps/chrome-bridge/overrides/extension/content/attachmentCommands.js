// Attachment preparation and upload commands for the extension content runtime.
// Loaded as a classic MV3 content script before content.js.
(() => {
  'use strict';

  // Composer-local evidence shared by upload waiting and interrupted-effect recovery.
  // A filename in page prose is not an attachment; image inputs need a loaded preview.
  function readUploadState(root, attachments, deps = {}) {
    const { isVisible = () => false, visibleText = () => '' } = deps;
    if (!root || root === document.body) return { known: false, count: 0, visibleNames: [], busy: false, error: '' };
    const nodes = Array.from(root.querySelectorAll('button.image-attachment, [role="button"].composer-attachment-surface, [data-testid*="attachment" i], [data-testid*="file" i]'));
    const visibleNodes = nodes.filter(isVisible);
    const visibleNames = [];
    const used = new Set();
    for (const item of Array.isArray(attachments) ? attachments : []) {
      const name = String(item?.name || '');
      if (!name) continue;
      const imageRequired = /^image\//i.test(String(item?.type || item?.mime || '')) || /\.(png|jpe?g|webp|gif|avif|bmp|heic)$/i.test(name);
      const match = visibleNodes.find(node => {
        if (used.has(node)) return false;
        const named = node.getAttribute('aria-label') === name
          || String(visibleText(node)).trim() === name
          || Array.from(node.querySelectorAll('[aria-label], [title]')).some(child => isVisible(child)
            && (child.getAttribute('aria-label') === name || child.getAttribute('title') === name));
        if (!named) return false;
        if (!imageRequired) return true;
        return Array.from(node.querySelectorAll('img')).some(img => isVisible(img)
          && Boolean(img.currentSrc || img.getAttribute('src'))
          && img.complete === true && img.naturalWidth > 0 && img.naturalHeight > 0);
      });
      if (match) { used.add(match); visibleNames.push(name); }
    }
    const text = String(visibleText(root));
    const error = text.match(/(upload failed|failed to upload|could not upload|unsupported file|file too large|ошибка загрузки|не удалось загрузить|файл слишком большой|上传失败|无法上传|不支持的文件|文件过大)/i)?.[0] || '';
    // Upload menu/attach buttons are controls, not progress evidence.
    const progress = Array.from(root.querySelectorAll('[aria-busy="true"], [role="progressbar"], progress, [data-testid*="progress" i], svg[class*="spinner" i], div[class*="spinner" i]'));
    const busy = root.getAttribute('aria-busy') === 'true' || progress.some(node => isVisible(node)
      && !/send|submit/i.test(node.getAttribute('data-testid') || ''));
    return { known: true, count: visibleNodes.length, visibleNames, busy, error };
  }

  function createAttachmentCommands(deps = {}) {
    const {
      CONFIG,
      EXTENSION_API,
      delay,
      diagnostic,
      emitChatEvent,
      findComposerRoot,
      findSendButton,
      isUsableButton,
      isVisible,
      send,
      visibleText,
    } = deps;

async function attachFiles(attachments, request) {
  emitChatEvent(request, 'files.attach.started', { count: attachments.length, files: attachments.map(stripAttachmentContent) });
  const files = [];

  for (const attachment of attachments) {
    try {
      files.push(await attachmentToFile(attachment));
      diagnostic('file.prepared', { requestId: request.requestId, id: attachment.id, name: attachment.name, size: attachment.size });
    } catch (err) {
      diagnostic('file.prepare_failed', { requestId: request.requestId, id: attachment.id, name: attachment.name, message: err.message });
      throw err;
    }
  }

  const input = await waitForFileInput(request);
  const dataTransfer = new DataTransfer();
  for (const file of files) dataTransfer.items.add(file);
  input.files = dataTransfer.files;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
  diagnostic('file.input.changed', { requestId: request.requestId, count: files.length, names: files.map((file) => file.name) });
  emitChatEvent(request, 'files.attach.changed', { count: files.length, names: files.map((file) => file.name) });

  try {
    await waitForAttachmentChips(files, request);
  } catch (err) {
    diagnostic('file.upload_wait.failed', { requestId: request.requestId, message: err.message });
    emitChatEvent(request, 'files.attach.failed', { message: err.message });
    throw err; // Missing parent input must never be followed by prompt submission.
  }
  emitChatEvent(request, 'files.attach.done', { count: files.length, names: files.map((file) => file.name) });
}

function stripAttachmentContent(attachment) {
  if (!attachment || typeof attachment !== 'object') return attachment;
  const { contentBase64, content, ...rest } = attachment;
  return rest;
}

async function attachmentToFile(attachment) {
  const name = String(attachment.name || attachment.filename || attachment.id || 'attachment');
  const mime = String(attachment.mime || attachment.type || 'application/octet-stream');

  let file;
  if (typeof attachment.contentBase64 === 'string') {
    const bytes = base64ToUint8Array(attachment.contentBase64);
    if (btoa(atob(attachment.contentBase64)) !== attachment.contentBase64) throw new Error('Attachment contains invalid base64');
    if (attachment.integrity && attachment.contentBase64.length !== attachment.integrity.encodedSize) throw new Error('Attachment encoded size mismatch');
    file = new File([bytes], name, { type: mime });
  } else if (attachment.content) {
    file = new File([String(attachment.content)], name, { type: mime || 'text/plain' });
  } else if (attachment.url) {
    const blob = await fetchAttachmentBlob(attachment.url, mime);
    file = new File([blob], name, { type: blob.type || mime });
  } else {
    throw new Error(`Attachment has no content: ${name}`);
  }
  if (attachment.integrity) {
    await globalThis.ChatGptTransferIntegrity.verifyBytes(await file.arrayBuffer(), attachment.integrity);
  } else if (Number.isSafeInteger(attachment.size) && attachment.size >= 0 && attachment.size !== file.size) {
    throw new Error('Attachment size mismatch');
  }
  return file;
}

function base64ToUint8Array(base64) {
  const binary = atob(String(base64 || ''));
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

async function fetchAttachmentBlob(url, fallbackMime = 'application/octet-stream') {
  try {
    const response = await fetch(url, { credentials: 'include' });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    return await response.blob();
  } catch (err) {
    if (typeof EXTENSION_API.httpRequest !== 'function') throw new Error(`Could not fetch attachment URL: ${err.message || err}`);
    return await new Promise((resolve, reject) => {
      EXTENSION_API.httpRequest({
        method: 'GET',
        url,
        responseType: 'blob',
        anonymous: false,
        onload(response) {
          if (response.status < 200 || response.status >= 300) {
            reject(new Error(`Could not fetch attachment URL through extension HTTP transport: HTTP ${response.status}`));
            return;
          }
          const blob = response.response instanceof Blob ? response.response : new Blob([response.response], { type: fallbackMime });
          resolve(blob);
        },
        onerror(response) {
          const detail = String(response?.error || '').trim();
          reject(new Error(`Could not fetch attachment URL through extension HTTP transport${detail ? `: ${detail}` : ''}`));
        },
        ontimeout() { reject(new Error('Timed out fetching attachment URL')); },
      });
    });
  }
}

async function waitForFileInput(request, timeoutMs = 10_000) {
  const existing = findFileInput();
  if (existing) return existing;

  const attachButton = findAttachButton();
  if (attachButton) {
    attachButton.click();
    diagnostic('file.attach_button.clicked', { requestId: request.requestId });
    await delay(350);
  } else {
    diagnostic('file.attach_button.not_found', { requestId: request.requestId });
  }

  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const input = findFileInput();
    if (input) return input;
    await delay(200);
  }

  throw new Error('File input not found in ChatGPT composer');
}

function findFileInput() {
  return Array.from(document.querySelectorAll('input[type="file"]')).find((input) => !input.disabled) || null;
}

function findAttachButton() {
  const buttons = Array.from(document.querySelectorAll('button, [role="button"]')).filter(isUsableButton);
  return buttons.find((button) => {
    const text = [button.getAttribute('aria-label'), button.getAttribute('title'), button.getAttribute('data-testid'), button.innerText || button.textContent || ''].filter(Boolean).join(' ');
    return /attach|upload|file|add photos|paperclip|прикреп|загруз|файл|скреп/i.test(text);
  }) || null;
}

async function waitForAttachmentChips(files, request) {
  const started = Date.now();
  const names = files.map((file) => file.name).filter(Boolean);
  let lastVisibleCount = -1;
  let lastProgressAt = -Infinity;
  let visibleCount = 0;

  while (Date.now() - started < CONFIG.attachmentUploadTimeoutMs) {
    const state = readUploadState(findComposerRoot(), files, { isVisible, visibleText });
    visibleCount = state.visibleNames.length;
    if (state.error) {
      diagnostic('file.upload_error', { requestId: request.requestId, message: state.error });
      throw new Error(`Attachment upload failed: ${state.error}`);
    }

    const sendButton = findSendButton();
    if (state.known && visibleCount === names.length && !state.busy && sendButton && isUsableButton(sendButton)) {
      diagnostic('file.upload.complete', { requestId: request.requestId, names, elapsedMs: Date.now() - started });
      return;
    }

    if (visibleCount !== lastVisibleCount || Date.now() - lastProgressAt >= 3000) {
      emitChatEvent(request, 'files.attach.progress', { visible: visibleCount, total: names.length, busy: state.busy });
      lastVisibleCount = visibleCount;
      lastProgressAt = Date.now();
    }
    await delay(350);
  }

  throw new Error(`Timed out waiting for file attachment upload completion (${visibleCount}/${names.length} visible)`);
}


function readComposerAttachmentState() {
  const root = findComposerRoot();
  if (!root || root === document.body) return { known: false, count: 0 };
  const buttons = findAttachmentRemoveButtons(root);
  return { known: true, count: buttons.length };
}

async function handleComposerAttachmentsClear(payload) {
  try {
    const result = await clearComposerAttachments();
    send({ type: 'composer.attachments.cleared', commandId: payload.commandId, ...result });
  } catch (err) {
    send({ type: 'command.error', commandId: payload.commandId, message: err.message || String(err) });
  }
}

async function clearComposerAttachments() {
  const root = findComposerRoot();
  const buttons = findAttachmentRemoveButtons(root);
  let removed = 0;
  for (const button of buttons) {
    try {
      button.click();
      removed += 1;
      await delay(120);
    } catch {
      // Continue with other candidates.
    }
  }
  diagnostic('composer.attachments.clear', { removed });
  return { removed, message: removed ? '' : 'No visible composer attachment remove buttons found' };
}

function findAttachmentRemoveButtons(root) {
  const candidates = Array.from((root || document.body).querySelectorAll('button, [role="button"]'));
  return candidates.filter((element) => {
    if (!isUsableButton(element)) return false;
    const attrs = [
      element.getAttribute('aria-label'),
      element.getAttribute('title'),
      element.getAttribute('data-testid'),
      visibleText(element),
    ].filter(Boolean).join(' ');
    if (!/(remove|delete|clear|close|dismiss|attachment|file|удал|убрать|очист|закры)/i.test(attrs)) return false;
    if (/send|submit|voice|microphone|settings|share|regenerate/.test(attrs.toLowerCase())) return false;
    return true;
  });
}

    return Object.freeze({ attachFiles, handleComposerAttachmentsClear, readComposerAttachmentState });
  }

  globalThis.ChatGptAttachmentCommands = Object.freeze({ createAttachmentCommands, readUploadState });
})();
