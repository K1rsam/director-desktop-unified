import { type Shape } from './shape.js';
export interface SafeNetworkRequest {
    headers?: Record<string, string>;
    body_kind?: 'empty' | 'json' | 'form' | 'opaque';
    body?: unknown;
    body_shape?: Shape;
    body_full_size?: number;
    body_truncated?: boolean;
    body_omitted?: boolean;
    redacted?: boolean;
}
export interface CapturedRequestMetadata {
    headers?: unknown;
    bodyKind?: unknown;
    bodyPreview?: unknown;
    bodyFullSize?: unknown;
    bodyTruncated?: unknown;
}
/**
 * Preserve request structure without turning `browser network` into a secret
 * dumper. Known credential fields are redacted. JSON objects and URL-encoded
 * forms remain inspectable; positional JSON and opaque bodies expose shape /
 * size only because their secret-bearing slots cannot be identified safely.
 */
export declare function sanitizeCapturedRequest(raw: CapturedRequestMetadata): SafeNetworkRequest | undefined;
/** Redact credential-shaped query parameters while preserving route identity. */
export declare function sanitizeCapturedUrl(rawUrl: string): string;
