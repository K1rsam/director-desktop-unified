import { ArtifactRegistry, publishArtifactSettlement } from './bridge/artifacts/artifactRegistry.js';
import { withAttachmentIntegrity } from './bridge/attachmentTransport.js';
import { randomUUID, createHash } from 'node:crypto';
import { config } from './config.js';
import { makeRequestId } from './protocol.js';
import { safeBridgeServerUrl } from './browserLaunch.js';
import { openExternalBrowserUrl } from './bridge/externalBrowser.js';
import {
  compactRequestState,
  makeEvent,
} from './bridge/requestState.js';
import { RequestEffectType } from './bridge/state/requestEvents.js';
import { BridgeOperations } from './bridge/coordinator/bridgeOperations.js';
import { RequestLifecycleCoordinator } from './bridge/coordinator/requestLifecycleCoordinator.js';
import { BridgeClientEventRouter } from './bridge/coordinator/bridgeClientEventRouter.js';
import { BrowserClientCoordinator } from './bridge/coordinator/browserClientCoordinator.js';
import { ObservedTurnJournal } from './bridge/observedTurns/observedTurnJournal.js';
import { BridgeCommandRegistry } from './bridge/coordinator/bridgeCommandRegistry.js';
import { RequestSubmissionCoordinator } from './bridge/coordinator/requestSubmissionCoordinator.js';
import { RequestControlCoordinator } from './bridge/coordinator/requestControlCoordinator.js';
import { PassivePromptService } from './bridge/passivePromptService.js';

export { browserLaunchUrl } from './browserLaunch.js';
export { openExternalBrowserUrl } from './bridge/externalBrowser.js';
export {
  removeCapturedBrowserDownload,
  resolveBrowserDownloadedPath,
} from './bridge/browserDownloads.js';

// Recovered image bytes can finish before the generic DOM observer finds its
// anchors. Feed verified native completion through the existing canonical
// lifecycle; never clear a lease, change a historical terminal, or send again.
export function nativeImageCompletionPayload(state, identity, client, options, response) {
  const expected = options?.reconcileConversation;
  const proof = response?.reconciliation;
  const id = expected?.requestId;
  const hash = value => createHash('sha256').update(value).digest('hex');
  const sha = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
  const branch = proof?.branchIdentity;
  const finalNode = branch?.[0];
  if (!state || state.runtime?.finished || state.runtime?.cancellationRequested
    || expected?.mode !== 'image_record_v1' || expected.imageCount !== 1
    || !/^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i.test(id || '')
    || state.requestId !== id || state.promptPayload?.requestId !== id
    || typeof state.promptPayload.message !== 'string' || !state.promptPayload.message.trim()
    || !sha(expected.promptSha256) || hash(state.promptPayload.message.split(/\s+/).filter(Boolean).join(' ')) !== expected.promptSha256
    || !identity?.leaseId || !identity.ownerServerInstanceId || identity.requestId !== id || identity.responseEpoch !== 0
    || state.clientId !== options.sourceClientId || client?.id !== state.clientId
    || client.ready !== true || client.compatible === false || client.quarantined === true
    || client.launchToken !== `bridge-director-desktop-${id}`
    || !['requestId', 'leaseId', 'ownerServerInstanceId', 'responseEpoch'].every(key => client.activeRequest?.[key] === identity[key])
    || response.sourceClientId !== state.clientId || response.requestId && response.requestId !== id
    || proof?.source !== 'conversation-record' || proof.version !== 1 || proof.proofVersion !== 2
    || proof.status !== 'matched_complete' || proof.requestId !== id || proof.promptSha256 !== expected.promptSha256
    || proof.conversationId !== expected.conversationId || proof.backendStatus !== 'finished_successfully' || proof.endTurn !== true
    || !proof.userMessageId || !proof.assistantMessageId || !proof.currentNode || !Array.isArray(branch) || !finalNode
    || branch.some(node => !node || typeof node !== 'object')
    || finalNode?.nodeKey !== proof.currentNode || finalNode.id !== proof.assistantMessageId
    || finalNode.role !== 'assistant' || finalNode.status !== 'finished_successfully' || finalNode.endTurn !== true
    || branch.filter(node => node.role === 'user').length !== 1
    || !branch.some(node => node.role === 'user' && node.id === proof.userMessageId)
    || !branch.some(node => ['assistant', 'tool'].includes(node.role) && node.nodeKey === proof.deliveryNode
      && node.id === proof.deliveryMessageId && node.status === 'finished_successfully')
    || response.session?.id !== proof.conversationId || response.turnKey !== proof.assistantMessageId
    || response.userTurnKey !== proof.userMessageId
    || !Array.isArray(proof.bindings) || proof.bindings.length !== 1
    || !Array.isArray(response.artifacts) || response.artifacts.length !== 1) return null;
  const conversationUrl = `https://chatgpt.com/c/${proof.conversationId}`;
  if (client.url !== conversationUrl || response.url !== conversationUrl) return null;
  const binding = proof.bindings[0], artifact = response.artifacts[0];
  if (!binding || !artifact || typeof binding !== 'object' || typeof artifact !== 'object') return null;
  let image;
  try { image = new URL(artifact.downloadUrl || artifact.src || artifact.url); } catch { return null; }
  const imageId = image.searchParams.get('id') || image.searchParams.get('file_id') || image.pathname.split('/content/')[1] || '';
  if (image.origin !== 'https://chatgpt.com' || !/^\/backend-api\/estuary\/content(?:\/|$)/.test(image.pathname)
    || imageId !== binding.imageFileId || !/^file[_-][a-zA-Z0-9_-]+$/.test(binding.imageFileId || '')
    || !sha(binding.bytesSha256) || !sha(binding.recordSha256) || !sha(binding.recordRecheckSha256)
    || !['native_record_download', 'dom_blob_bytes_equal', 'dom_native_url_download'].includes(binding.proofSource)
    || !Number.isSafeInteger(binding.size) || binding.size < 12 || binding.size > 20 * 1024 * 1024 || !['image/png', 'image/jpeg', 'image/webp'].includes(binding.mime)
    || artifact.kind !== 'image' || artifact.generatedImage !== true || artifact.downloadable !== true
    || !artifact.id || artifact.id !== binding.artifactId || artifact.size !== binding.size || artifact.mime !== binding.mime
    || artifact.sourceClientId !== state.clientId || artifact.sourceTurnKey !== proof.assistantMessageId
    || !['', id, 'recovered'].includes(artifact.requestId || '')) return null;
  return { ...response, requestId: id,
    artifacts: [{ ...artifact, requestId: id }],
    finishReason: 'native_image_record_complete',
    completionEvidence: { generationStopped: true, finalMessage: true, nativeProof: proof } };
}

export class BrowserBridge {
  #hub;
  #fileStore;
  #eventBus;
  #pending = new Map();
  #artifacts;
  #observedTurnJournal = new ObservedTurnJournal({ limit: 200 });
  #lifecycle;
  #browserClients;
  #clientEvents;
  #operations;
  #commandRegistry;
  #submission;
  #controls;
  #passivePrompts;
  #runtimeOptions;
  #serverInstanceId;

  constructor(hub, fileStore = null, eventBus = null, runtimeOptions = {}) {
    this.#hub = hub;
    this.#serverInstanceId = String(runtimeOptions.serverInstanceId || hub?.serverInstanceId || randomUUID());
    this.#fileStore = fileStore;
    this.#eventBus = eventBus;
    this.#runtimeOptions = {
      autoOpenTab: typeof runtimeOptions.autoOpenTab === 'boolean' ? runtimeOptions.autoOpenTab : config.autoOpenTab,
      autoOpenTabTimeoutMs: Math.max(5_000, Number(runtimeOptions.autoOpenTabTimeoutMs) || config.autoOpenTabTimeoutMs),
      autoOpenTabBootstrapWaitMs: Math.max(0, Number(runtimeOptions.autoOpenTabBootstrapWaitMs ?? config.autoOpenTabBootstrapWaitMs) || 0),
      passivePromptReviewAfterMs: Math.max(1_000, Number(runtimeOptions.passivePromptReviewAfterMs ?? config.passivePromptReviewAfterMs) || config.passivePromptReviewAfterMs),
      passivePromptNow: typeof runtimeOptions.passivePromptNow === 'function' ? runtimeOptions.passivePromptNow : () => Date.now(),
      openExternalUrl: typeof runtimeOptions.openExternalUrl === 'function' ? runtimeOptions.openExternalUrl : openExternalBrowserUrl,
      publicBaseUrl: safeBridgeServerUrl(runtimeOptions.publicBaseUrl || config.publicBaseUrl),
    };
    this.#commandRegistry = new BridgeCommandRegistry({ hub: this.#hub, eventBus: this.#eventBus });
    this.#artifacts = new ArtifactRegistry({
      capture: (id) => this.#operations.fetchArtifact(id),
      onSettled: (artifact) => publishArtifactSettlement(artifact, {
        pending: this.#pending, lifecycle: this.#lifecycle, eventBus: this.#eventBus,
      }),
    });
    this.#operations = new BridgeOperations({
      sendCommand: async (type, data, options) => await this.#sendCommand(type, data, options),
      fileStore: this.#fileStore,
      eventBus: this.#eventBus,
      artifacts: this.#artifacts,
    });
    this.#passivePrompts = new PassivePromptService({
      operations: this.#operations,
      metadataStore: runtimeOptions.metadataStore || null,
      now: this.#runtimeOptions.passivePromptNow,
      reviewAfterMs: this.#runtimeOptions.passivePromptReviewAfterMs,
    });
    this.#lifecycle = new RequestLifecycleCoordinator({
      hub: this.#hub,
      pending: this.#pending,
      artifacts: this.#artifacts,
      eventBus: this.#eventBus,
      sendCommand: async (type, data, options) => await this.#sendCommand(type, data, options),
      resumePrompt: async (sourceClientId, payload, options = {}) => {
        const client = Array.from(this.#hub.clients || []).find((candidate) => candidate.id === sourceClientId);
        if (!client) throw new Error(`Browser extension client not found for prompt recovery: ${sourceClientId}`);
        const state = this.#pending.get(String(payload.requestId || ''));
        const sent = this.#browserClients.sendPromptToClient(client, payload, {
          ...options,
          request: this.#lifecycle.requestIdentity(state, payload.responseEpoch),
        });
        return await sent.delivered;
      },
    });
    this.#browserClients = new BrowserClientCoordinator({
      hub: this.#hub,
      pending: this.#pending,
      lifecycle: this.#lifecycle,
      runtimeOptions: this.#runtimeOptions,
      sendCommand: async (type, data, options) => await this.#sendCommand(type, data, options),
      releaseCoordinator: this.#commandRegistry,
    });
    this.#submission = new RequestSubmissionCoordinator({
      pending: this.#pending,
      lifecycle: this.#lifecycle,
      browserClients: this.#browserClients,
      eventBus: this.#eventBus,
      hub: this.#hub,
      serverInstanceId: this.#serverInstanceId,
      sendCommand: async (type, data, options) => await this.#sendCommand(type, data, options),
      resolveAttachments: async (attachments) => await this.#resolveAttachments(attachments),
    });
    this.#controls = new RequestControlCoordinator({
      pending: this.#pending,
      lifecycle: this.#lifecycle,
      operations: this.#operations,
      sendCommand: async (type, data, options) => await this.#sendCommand(type, data, options),
    });
    this.#clientEvents = new BridgeClientEventRouter({
      pending: this.#pending,
      commands: this.#commandRegistry.commands,
      artifacts: this.#artifacts,
      lifecycle: this.#lifecycle,
      eventBus: this.#eventBus,
      publishObservedTurn: (turn) => this.#publishObservedTurn(turn),
      registerObservedArtifacts: (artifacts, defaults) => this.registerObservedArtifacts(artifacts, defaults),
      handleCommandResponse: (clientId, payload) => this.#commandRegistry.handleResponse(clientId, payload),
      sendCommand: async (type, data, options) => await this.#sendCommand(type, data, options),
    });
    const canonicalHandler = async ({ eventName, data }) => {
      if (eventName === 'client.message') return await this.#clientEvents.handleClientMessage(data.clientId, data.payload, data.envelope);
      if (eventName === 'client.activity') return await this.#clientEvents.handleClientActivity(data.clientId, data.client, data.payload, data.envelope);
      return undefined;
    };
    if (typeof this.#hub.setCanonicalMessageHandler === 'function') this.#hub.setCanonicalMessageHandler(canonicalHandler);
    else {
      this.#hub.on?.('client.message', (data) => canonicalHandler({ eventName: 'client.message', data }));
      this.#hub.on?.('client.activity', (data) => canonicalHandler({ eventName: 'client.activity', data }));
    }
    this.#hub.on?.('client.ready', (client) => this.#clientEvents.handleClientReady(client));
    this.#hub.on?.('client.changed', (client) => this.#clientEvents.handleClientChanged(client));
    this.#hub.on?.('client.closed', (client) => this.#lifecycle.handleClientClosed(client));
  }

  get pageUrl() {
    return this.#hub.activeClient?.url || null;
  }

  canAutoOpenPromptTab(options = {}) {
    return this.#browserClients.canAutoOpenPromptTab(options);
  }

  activeRequestCandidates() {
    return this.#browserClients.activeRequestCandidates();
  }

  findActiveRequest(options = {}) {
    return this.#browserClients.findActiveRequest(options);
  }

  async openBrowserTab(options = {}) {
    return await this.#browserClients.openBrowserTab(options);
  }

  async closeBrowserTab(options = {}) {
    return await this.#browserClients.closeBrowserTab(options);
  }

  async closeOwnedBrowserTab(options = {}) { return await this.#browserClients.closeOwnedBrowserTab(options); }

  async reloadExtension(options = {}) {
    return await this.#browserClients.reloadExtension(options);
  }

  async connectBrowser() {
    if (!this.#hub.activeClient) {
      const incompatibleClients = Array.from(this.#hub.clients || []).filter((client) => client.compatible === false || client.compatibility?.compatible === false);
      if (incompatibleClients.length) {
        const details = incompatibleClients.map((client) => `${client.id}: ${client.compatibility?.message || 'extension update required'}`).join('; ');
        throw new Error(`Connected browser extension is incompatible. ${details}`);
      }
      throw new Error('No browser extension client connected. Open ChatGPT with the ChatGPT Bridge extension enabled.');
    }
  }

  health() {
    const active = this.#hub.activeClient;
    const clients = Array.from(this.#hub.clients || []).map((client) => {
      const releasePending = this.#commandRegistry.isReleasePending(client.id);
      return {
        ...client,
        releasePending,
        releaseState: client.quarantined ? 'quarantined' : releasePending ? 'pending' : 'idle',
      };
    });
    return {
      ok: Boolean(active),
      transport: active ? `${active.runtime === 'extension' || active.transport === 'extension' ? 'extension' : 'browser'}:${active.transport || 'unknown'}` : 'extension:disconnected',
      clients,
      activeClient: active ? clients.find((client) => client.id === active.id) : null,
      selectedClientId: this.#hub.selectedClientId,
      needsSelection: this.#hub.needsSelection,
      pendingRequests: this.#pending.size,
      pendingCommands: this.#commandRegistry.size,
      artifacts: this.#artifacts.size,
      activeRequests: this.requestDiagnostics(),
      canonicalRequestState: {
        enabled: true,
        tracked: this.#lifecycle.trackedCount(),
        authoritativeLifecycle: true,
        extensionProtocol: 'v5-only',
        scheduledDeadlines: this.#lifecycle.deadlines().length,
      },
      serverInstanceId: this.#serverInstanceId,
      autoOpenTab: Boolean(this.#runtimeOptions.autoOpenTab),
    };
  }

  requestDiagnostics() {
    return Array.from(this.#pending.values()).map((state) => {
      const canonicalState = this.#lifecycle.snapshot(state.requestId);
      const observationData = canonicalState?.lastObservation?.data || null;
      const observation = observationData?.observation || null;
      const activeRequest = observation?.activeRequest || null;
      return {
        ...compactRequestState(state, canonicalState),
        phase: canonicalState?.displayPhase || canonicalState?.lifecycle || state.progress?.phase || 'unknown',
        sourceUrl: canonicalState?.source?.url || state.progress?.url || '',
        sourceSession: canonicalState?.source?.conversationId
          ? { id: canonicalState.source.conversationId }
          : state.progress?.session || state.session || null,
        submittedUserTurnKey: String(activeRequest?.submittedUserTurnKey || canonicalState?.response?.userTurnKey || ''),
        submittedUserTurnIndex: Number(activeRequest?.submittedUserTurnIndex ?? -1),
        assistantTurnKey: String(observationData?.turnKey || activeRequest?.assistantTurnKey || ''),
        assistantTurnIndex: Number(observationData?.turnIndex ?? activeRequest?.assistantTurnIndex ?? -1),
        currentGenerationActive: canonicalState?.generation === 'active',
        canonicalState,
        canonicalDeadlines: this.#lifecycle.deadlines(state.requestId),
      };
    });
  }

  requestStateDiagnostics(requestId = '') {
    return this.#lifecycle.diagnostics(requestId);
  }

  async requestForcedSnapshot(requestId, options = {}) {
    const state = this.#pending.get(String(requestId || ''));
    if (!state) throw new Error(`No pending request for forced snapshot: ${requestId}`);
    return await this.#lifecycle.runRequestEffect(state, {
      id: `${state.requestId}:manual-snapshot:${Date.now()}`,
      type: RequestEffectType.RESPONSE_SNAPSHOT,
      data: { reason: options.reason || 'manual_forced_snapshot', manual: true },
      execute: async () => await this.#lifecycle.requestForcedSnapshotForState(
        state,
        options.reason || 'manual_forced_snapshot',
        { manual: true, force: true },
      ),
    });
  }


  async steerRequest(requestId, message, options = {}) {
    return await this.#controls.steerRequest(requestId, message, options);
  }

  selectClient(clientId) {
    return this.#hub.selectClient(clientId);
  }

  async identifyBrowserTab(clientId = '', options = {}) {
    const sourceClientId = String(clientId || options.sourceClientId || this.#hub.activeClient?.id || '').trim();
    if (!sourceClientId) throw new Error('No active browser tab is available to identify');
    return await this.#operations.identifyBrowserTab({ ...options, sourceClientId });
  }

  clearSelectedClient() {
    this.#hub.clearSelectedClient();
  }

  dropClient(clientId) {
    return this.#hub.dropClient(clientId);
  }

  setClientQuarantineForE2E(clientId, options = {}) {
    return this.#hub.setClientQuarantineForE2E(clientId, options);
  }


  debugEvents() {
    return this.#hub.debugEvents;
  }

  onClientLifecycle(handler) {
    if (typeof handler !== 'function') return () => {};
    const events = ['client.ready', 'client.changed', 'client.closed'];
    for (const event of events) this.#hub.on(event, handler);
    return () => {
      for (const event of events) this.#hub.off(event, handler);
    };
  }


  validateBridgeToken(token) {
    return this.#hub.validateToken(token);
  }

  isLocalRequest(req) {
    return this.#hub.isLocalRequest(req);
  }

  listKnownArtifacts() {
    return Array.from(this.#artifacts.values());
  }

  cancelActive(reason = 'Cancelled by user') {
    const pending = Array.from(this.#pending.values());
    for (const state of pending) {
      this.#lifecycle.cancelState(state, reason);
    }
    return pending.length;
  }


  async resumeActiveRequest(callbacks = {}, options = {}) {
    return await this.#submission.resumeActiveRequest(callbacks, options);
  }

  async sendToChatGPT(message, callbacks = {}, options = {}) {
    return await this.#submission.sendToChatGPT(message, callbacks, options);
  }

  async sendRequest(request, callbacks = {}, options = {}) {
    return await this.#submission.sendRequest(request, callbacks, options);
  }

  async deleteSession(sessionId, expectedUrl, options = {}) {
    return await this.#operations.deleteSession(sessionId, expectedUrl, options);
  }

  async listSessions(options = {}) {
    return await this.#operations.listSessions(options);
  }

  async newSession(options = {}) {
    return await this.#operations.newSession(options);
  }

  async selectSession(sessionId, options = {}) {
    return await this.#operations.selectSession(sessionId, options);
  }

  async listModels(options = {}) {
    return await this.#operations.listModels(options);
  }

  async listEfforts(options = {}) {
    return await this.#operations.listEfforts(options);
  }

  async applyIntelligence(values = {}, options = {}) {
    return await this.#operations.applyIntelligence(values, options);
  }

  async clearComposerAttachments(options = {}) {
    return await this.#operations.clearComposerAttachments(options);
  }

  async recoverResponses(options = {}) {
    return await this.#operations.recoverResponses(options);
  }

  async recoverLatestResponse(options = {}) {
    const response = await this.#operations.recoverLatestResponse(options);
    const state = this.#pending.get(String(options.reconcileConversation?.requestId || ''));
    if (state) {
      const clients = Array.from(this.#hub.clients || []).filter(client => client.id === state.clientId);
      const payload = clients.length === 1 ? nativeImageCompletionPayload(
        state, this.#lifecycle.requestIdentity(state), clients[0], options, response,
      ) : null;
      if (payload) this.#lifecycle.ingestTerminalPayload(state, state.clientId, payload, 'native_image_record_complete');
    }
    return response;
  }

  async recoverResponseByTurnKey(options = {}) {
    return await this.#operations.recoverResponseByTurnKey(options);
  }

  async fetchArtifact(artifactId, options = {}) {
    return await this.#operations.fetchArtifact(artifactId, options);
  }

  onObservedTurn(listener) { return this.#observedTurnJournal.onTurn(listener); }

  onObservedTurnEnvelope(listener) { return this.#observedTurnJournal.onEnvelope(listener); }

  listObservedTurns(options = {}) { return this.#observedTurnJournal.list(options); }

  observedTurnStreamState(options = {}) { return { ...this.#observedTurnJournal.classifyCursor(options), serverInstanceId: this.#serverInstanceId }; }

  registerObservedArtifacts(artifacts = [], metadata = {}) {
    return this.#operations.registerObservedArtifacts(artifacts, metadata);
  }

  async submitPassivePrompt(options = {}) {
    return await this.#passivePrompts.submit(options);
  }

  async getPassivePromptStatus(requestId = '') {
    return await this.#passivePrompts.status(requestId);
  }

  async reconcilePassivePrompt(requestId = '', options = {}) {
    return await this.#passivePrompts.reconcileOwnerNotSent(requestId, options);
  }

  async reloadBrowserTab(options = {}) {
    return await this.#controls.reloadBrowserTab(options);
  }

  async capturePageLayout(options = {}) {
    const sourceClientId = String(options.sourceClientId || options.clientId || '').trim();
    const active = sourceClientId ? this.findActiveRequest({ sourceClientId }) : null;
    return await this.#operations.capturePageLayout({
      ...options,
      sourceClientId,
      requestId: String(options.requestId || active?.activeRequest?.requestId || ''),
    });
  }

  async close({ cancelPending = true } = {}) {
    if (cancelPending) {
      for (const state of this.#pending.values()) {
        this.#lifecycle.cancelState(state, 'Bridge shutting down');
      }
    }
    this.#pending.clear();
    this.#lifecycle.close();

    this.#commandRegistry.close('Bridge shutting down');
  }

  #publishObservedTurn(turn = {}) {
    if (turn.artifacts?.some((artifact) => artifact.phase === 'MATERIALIZING')) {
      // Do not await on the inbound Protocol 5 queue: capture results use it too.
      void this.#artifacts.settled(turn.artifacts).then((artifacts) => this.#publishObservedTurn({ ...turn, artifacts }));
      return null;
    }
    const envelope = this.#observedTurnJournal.publish(turn);
    this.#eventBus?.emitDebug?.({
      type: 'watch.turn.journaled',
      data: { streamEpoch: envelope.streamEpoch, sequence: envelope.sequence, turnKey: envelope.turn?.turnKey || '' },
    });
    return envelope;
  }

  async #resolveAttachments(rawAttachments) {
    const result = [];
    for (const raw of rawAttachments) {
      if (!raw) continue;
      if (typeof raw === 'string') {
        if (!this.#fileStore) throw new Error('FileStore is not configured');
        result.push(await this.#readAttachmentForTransport(raw));
        continue;
      }

      if (typeof raw === 'object') {
        const fileId = raw.fileId || raw.id;
        if (fileId && !raw.contentBase64 && !raw.content && this.#fileStore) {
          result.push(await this.#readAttachmentForTransport(fileId));
          continue;
        }
        if (raw.url && !raw.contentBase64 && !raw.content) {
          result.push({ id: raw.id || raw.fileId || `url_${makeRequestId()}`, name: raw.name || 'attachment',
            mime: raw.mime || raw.type || 'application/octet-stream',
            ...(raw.size === undefined ? {} : { size: raw.size }),
            sha256: raw.sha256 || '',
            url: raw.url,
          });
          continue;
        }
        if (raw.contentBase64 || raw.content) {
          result.push({
            id: raw.id || raw.fileId || `inline_${makeRequestId()}`,
            name: raw.name || 'attachment',
            mime: raw.mime || raw.type || 'application/octet-stream',
            contentBase64: raw.contentBase64 || Buffer.from(String(raw.content || ''), 'utf8').toString('base64'),
          });
        }
      }
    }
    return result.map(withAttachmentIntegrity);
  }

  async #readAttachmentForTransport(fileId) {
    const record = await this.#fileStore.get(fileId);
    if (!record) throw new Error(`File not found: ${fileId}`);
    if (config.attachmentTransport === 'base64') return await this.#fileStore.readForTransport(fileId);
    const url = new URL(`/extension/files/${encodeURIComponent(fileId)}/download`, this.#runtimeOptions.publicBaseUrl);
    return { id: record.id, name: record.name,
      mime: record.mime || 'application/octet-stream',
      size: record.size,
      sha256: record.sha256,
      url: url.toString(),
    };
  }

  #sendCommand(type, payload = {}, options = {}) {
    return this.#commandRegistry.send(type, payload, options);
  }
}
