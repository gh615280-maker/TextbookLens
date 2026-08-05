import type { ContentAnchor } from '../../lib/generated/document';

export const LEARNING_ACTIONS = Object.freeze([
  'explain',
  'example',
  'derive',
  'translate',
  'ask',
] as const);
export type LearningAction = (typeof LEARNING_ACTIONS)[number];

export const LEARNING_CONTENT_KINDS = Object.freeze([
  'text_selection',
  'reliable_text_region',
  'visual_region',
] as const);
export type LearningContentKind = (typeof LEARNING_CONTENT_KINDS)[number];

export const PREPARATION_RISK_FLAGS = Object.freeze([
  'image_send',
  'cost_risk',
] as const);
export type PreparationRiskFlag = (typeof PREPARATION_RISK_FLAGS)[number];

export const LEARNING_AUTHORIZATION_DECISIONS = Object.freeze([
  'allow',
  'deny',
] as const);
export type LearningAuthorizationDecision =
  (typeof LEARNING_AUTHORIZATION_DECISIONS)[number];

export const LEARNING_INVALIDATION_REASONS = Object.freeze([
  'route_change',
  'book_change',
  'profile_change',
  'default_change',
] as const);
export type LearningInvalidationReason =
  (typeof LEARNING_INVALIDATION_REASONS)[number];

export const LEARNING_ERROR_CODES = Object.freeze([
  'CONTEXT_TOO_LARGE',
  'CREDENTIAL_STORE_ERROR',
  'INVALID_INPUT',
  'NOT_FOUND',
  'BOOK_NOT_READY',
  'REQUEST_CONFLICT',
  'DATABASE_ERROR',
  'UNSUPPORTED_PROVIDER_CAPABILITY',
] as const);
export type LearningErrorCode = (typeof LEARNING_ERROR_CODES)[number];

export interface LearningError {
  readonly code: LearningErrorCode;
}

export interface PrepareLearningRequestMetadata {
  readonly bookId: string;
  readonly sectionId: string;
  readonly providerProfileId: string;
  readonly modelId: string;
  readonly action: LearningAction;
  readonly contentKind: LearningContentKind;
  readonly anchor: ContentAnchor;
  readonly selectedText: string | null;
  readonly question: string | null;
  readonly targetLanguage: string | null;
}

export interface PreparationSummary {
  readonly preparationId: string;
  readonly providerDisplayName: string;
  readonly profileDisplayName: string;
  readonly modelDisplayName: string;
  readonly estimatedInputTokens: number;
  readonly sourceCount: number;
  readonly citationCount: number;
  readonly omittedSourceCount: number;
  readonly willSendImage: boolean;
  readonly riskFlags: readonly PreparationRiskFlag[];
  readonly requiresBlockingConfirmation: boolean;
  readonly expiresAt: string;
  readonly actionCategory: LearningAction;
}

export interface RegionCaptureMetadata {
  readonly preparationId: string;
  readonly operationToken: string;
  readonly bookId: string;
  readonly providerProfileId: string;
  readonly modelId: string;
  readonly anchorContentSha256: string;
  readonly captureSha256: string;
  readonly schemaVersion: 1;
  readonly mimeType: 'image/png';
  readonly width: number;
  readonly height: number;
  readonly decodedPixelCount: number;
  readonly encodedByteLength: number;
}

export interface InvalidateLearningPreparations {
  readonly reason: LearningInvalidationReason;
  readonly bookId: string | null;
  readonly providerProfileId: string | null;
}

export function freezePreparationMetadata(
  metadata: PrepareLearningRequestMetadata,
): Readonly<PrepareLearningRequestMetadata> {
  return deepFreeze(metadata);
}

export function freezePreparationSummary(
  summary: PreparationSummary,
): Readonly<PreparationSummary> {
  return deepFreeze(summary);
}

export function freezeRegionCaptureMetadata(
  metadata: RegionCaptureMetadata,
): Readonly<RegionCaptureMetadata> {
  return deepFreeze(metadata);
}

function deepFreeze<T>(value: T): T {
  if (value !== null && typeof value === 'object' && !Object.isFrozen(value)) {
    for (const nested of Object.values(value)) deepFreeze(nested);
    Object.freeze(value);
  }
  return value;
}
