/** Director private protocol; these are NOT official Codex SDK protocol types. */
export type ProtocolVersion = 'director-sdk-bridge/v1';
export type BridgeMethod = 'hello' | 'inspect' | 'execute' | 'observe' | 'cancel' | 'shutdown';
export type FrozenAttempt = FrozenAttemptFields & (
  { version: 'text-attempt/v2'; compiledInputFormat?: never }
  | { version: 'text-attempt/v3'; compiledInputFormat: 'rules-first-json/v1' }
);
interface FrozenAttemptFields {
  attemptId: string;
  workflowId: string;
  logicalNodeId: string;
  backend: 'codex_sdk';
  request: { stage: string; skill: string; model: string; reasoning: string; input: unknown; schema: unknown; ruleSnapshot: unknown };
  semanticRequestHash: string;
  executionFingerprint: string;
  runtimeIdentity?: unknown;
  state: 'prepared';
  threadId: null;
  turnId: null;
  lastSequence: number;
  rawResult: null;
}
export interface ExecuteParams {
  attempt: FrozenAttempt;
  compiledPrompt: string;
  compiledSchema: string;
  compiledPromptSha256: string;
  compiledSchemaSha256: string;
}
export interface BridgeRequest {
  protocolVersion: ProtocolVersion;
  requestId: string;
  attemptId: string | null;
  method: BridgeMethod;
  params?: ExecuteParams;
}
export interface RawResult { rawText: string; parsedJson: unknown | null; parseError: string | null; domainAccepted: false }
export interface Receipt {
  attemptId: string;
  semanticRequestHash: string;
  executionFingerprint: string;
  attemptEnvelopeHash: string | null;
  compiledPromptSha256: string;
  compiledSchemaSha256: string;
  backend: 'codex_sdk';
  state: 'dispatching' | 'running' | 'cancel_requested' | 'submission_unknown' | 'transport_completed';
  threadId: string | null;
  turnId: null;
  lastSequence: number;
  rawResult: RawResult | null;
  usage: Record<string, number> | null;
  error: { code: string; message: string } | null;
  domainAccepted: false;
  processTreeStopProven: false;
  remoteObservationSupported: false;
  receiptRelativePath: string;
}
