import { invoke } from '@tauri-apps/api/core';
import { z } from 'zod';

import type {
  BookLearningPreparationSummary,
  PrepareBookLearningRequestMetadata,
} from '../../lib/generated/book_learning';
import type { LearningRequestSnapshot } from '../../lib/generated/panel';
import { parseRequestSnapshot } from './api';
import {
  LEARNING_AUTHORIZATION_DECISIONS,
  LEARNING_ERROR_CODES,
  type LearningAuthorizationDecision,
  type LearningError,
  type LearningErrorCode,
} from './learning-contract';

const MAX_QUESTION_CODE_POINTS = 16_384;
const MAX_QUESTION_BYTES = 64 * 1024;
const STANDARD_CONTEXT_CAP = 32_000;
const MAX_DISPLAY_NAME_CODE_POINTS = 512;
const ISO_TIMESTAMP =
  /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/u;

const uuidSchema = z.uuid();
const uint32Schema = z.number().int().min(0).max(0xffff_ffff);
const boundedQuestionSchema = z.string().superRefine((value, context) => {
  if (
    value.trim().length === 0 ||
    Array.from(value).length > MAX_QUESTION_CODE_POINTS ||
    new TextEncoder().encode(value).byteLength > MAX_QUESTION_BYTES ||
    hasDisallowedUnicode(value)
  ) {
    context.addIssue({ code: 'custom', message: 'invalid question' });
  }
});
const boundedDisplayNameSchema = z.string().superRefine((value, context) => {
  if (
    value.trim().length === 0 ||
    Array.from(value).length > MAX_DISPLAY_NAME_CODE_POINTS ||
    hasDisallowedUnicode(value)
  ) {
    context.addIssue({ code: 'custom', message: 'invalid display name' });
  }
});
const prepareBookMetadataSchema = z.discriminatedUnion('kind', [
  z
    .object({
      kind: z.literal('new'),
      bookId: uuidSchema,
      question: boundedQuestionSchema,
    })
    .strict(),
  z
    .object({
      kind: z.literal('continue'),
      bookId: uuidSchema,
      conversationId: uuidSchema,
      question: boundedQuestionSchema,
    })
    .strict(),
]);
const bookSummarySchema = z
  .object({
    preparationId: uuidSchema,
    providerDisplayName: boundedDisplayNameSchema,
    profileDisplayName: boundedDisplayNameSchema,
    modelDisplayName: boundedDisplayNameSchema,
    estimatedInputTokens: uint32Schema,
    sourceCount: uint32Schema,
    citationCount: uint32Schema,
    omittedSourceCount: uint32Schema,
    riskFlags: z.array(z.literal('cost_risk')).max(1),
    requiresBlockingConfirmation: z.boolean(),
    expiresAt: z.string().regex(ISO_TIMESTAMP),
  })
  .strict()
  .superRefine((summary, context) => {
    const hasCostRisk = summary.riskFlags.includes('cost_risk');
    if (
      new Set(summary.riskFlags).size !== summary.riskFlags.length ||
      hasCostRisk !== summary.estimatedInputTokens > STANDARD_CONTEXT_CAP ||
      (summary.requiresBlockingConfirmation && !hasCostRisk)
    ) {
      context.addIssue({ code: 'custom', message: 'inconsistent summary' });
    }
  });
const learningErrorCodeSchema = z.enum(LEARNING_ERROR_CODES);

export interface BookLearningPreparationApi {
  prepare(
    metadata: PrepareBookLearningRequestMetadata,
  ): Promise<Readonly<BookLearningPreparationSummary>>;
  authorize(
    preparationId: string,
    decision: LearningAuthorizationDecision,
  ): Promise<void>;
  discard(preparationId: string): Promise<void>;
  start(preparationId: string): Promise<LearningRequestSnapshot>;
}

export class TauriBookLearningPreparationApi implements BookLearningPreparationApi {
  async prepare(metadata: PrepareBookLearningRequestMetadata) {
    let safeMetadata: Readonly<PrepareBookLearningRequestMetadata>;
    try {
      safeMetadata = deepFreeze(
        prepareBookMetadataSchema.parse(
          metadata,
        ) as PrepareBookLearningRequestMetadata,
      );
    } catch {
      throw learningError('INVALID_INPUT');
    }

    let response: unknown;
    try {
      response = await invoke<unknown>('prepare_book_learning_request', {
        metadata: safeMetadata,
      });
    } catch (error) {
      throw invocationError(error);
    }
    try {
      return deepFreeze(
        bookSummarySchema.parse(response) as BookLearningPreparationSummary,
      );
    } catch {
      throw learningError('DATABASE_ERROR');
    }
  }

  async authorize(
    preparationId: string,
    decision: LearningAuthorizationDecision,
  ): Promise<void> {
    let args: {
      preparationId: string;
      decision: LearningAuthorizationDecision;
    };
    try {
      args = {
        preparationId: uuidSchema.parse(preparationId),
        decision: z.enum(LEARNING_AUTHORIZATION_DECISIONS).parse(decision),
      };
    } catch {
      throw learningError('INVALID_INPUT');
    }
    try {
      await invoke('authorize_book_learning_request', args);
    } catch (error) {
      throw invocationError(error);
    }
  }

  async discard(preparationId: string): Promise<void> {
    let id: string;
    try {
      id = uuidSchema.parse(preparationId);
    } catch {
      throw learningError('INVALID_INPUT');
    }
    try {
      await invoke('discard_book_learning_preparation', {
        preparationId: id,
      });
    } catch (error) {
      throw invocationError(error);
    }
  }

  async start(preparationId: string): Promise<LearningRequestSnapshot> {
    let id: string;
    try {
      id = uuidSchema.parse(preparationId);
    } catch {
      throw learningError('INVALID_INPUT');
    }
    let response: unknown;
    try {
      response = await invoke<unknown>('start_book_learning_request', {
        preparationId: id,
      });
    } catch (error) {
      throw invocationError(error);
    }
    try {
      return parseRequestSnapshot(response);
    } catch {
      throw learningError('DATABASE_ERROR');
    }
  }
}

function hasDisallowedUnicode(value: string): boolean {
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    if (
      codePoint === undefined ||
      (codePoint <= 0x1f && codePoint !== 0x09 && codePoint !== 0x0a) ||
      (codePoint >= 0x7f && codePoint <= 0x9f) ||
      (codePoint >= 0xd800 && codePoint <= 0xdfff)
    ) {
      return true;
    }
  }
  return false;
}

function invocationError(error: unknown): Readonly<LearningError> {
  const parsed = z
    .object({ code: learningErrorCodeSchema })
    .passthrough()
    .safeParse(error);
  return learningError(parsed.success ? parsed.data.code : 'DATABASE_ERROR');
}

function learningError(code: LearningErrorCode): Readonly<LearningError> {
  return Object.freeze({ code });
}

function deepFreeze<T>(value: T): T {
  if (value !== null && typeof value === 'object' && !Object.isFrozen(value)) {
    for (const nested of Object.values(value)) deepFreeze(nested);
    Object.freeze(value);
  }
  return value;
}
