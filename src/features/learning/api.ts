import { Channel, invoke } from '@tauri-apps/api/core';
import { z } from 'zod';

import {
  LEARNING_ACTIONS,
  LEARNING_AUTHORIZATION_DECISIONS,
  LEARNING_CONTENT_KINDS,
  LEARNING_ERROR_CODES,
  LEARNING_INVALIDATION_REASONS,
  PREPARATION_RISK_FLAGS,
  freezePreparationMetadata,
  freezePreparationSummary,
  freezeRegionCaptureMetadata,
  type InvalidateLearningPreparations,
  type LearningAuthorizationDecision,
  type LearningError,
  type LearningErrorCode,
  type PreparationSummary,
  type PrepareLearningRequestMetadata,
  type RegionCaptureMetadata,
} from './learning-contract';
import type {
  LearningRequestEvent,
  LearningRequestSnapshot,
} from '../../lib/generated/panel';

const CAPTURE_METADATA_HEADER = 'x-textbooklens-learning-capture';
const MAX_CAPTURE_BYTES = 4 * 1024 * 1024;
const MAX_CAPTURE_DIMENSION = 4_096;
const MAX_CAPTURE_PIXELS = 8_847_360;
const MAX_PDF_RECT_PAGES = 1_024;
const MAX_PDF_RECTS_TOTAL = 1_024;
const MAX_SELECTED_TEXT_CODE_POINTS = 1_048_576;
const MAX_QUESTION_CODE_POINTS = 16_384;
const MAX_MODEL_ID_CODE_POINTS = 512;
const LOWER_SHA256 = /^[0-9a-f]{64}$/u;
const ISO_TIMESTAMP =
  /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/u;

const uuidSchema = z.uuid();
const sha256Schema = z.string().regex(LOWER_SHA256);
const uint32Schema = z.number().int().min(0).max(0xffff_ffff);
const positiveUint32Schema = uint32Schema.min(1);
const boundedText = (maximum: number) =>
  z
    .string()
    .max(maximum)
    .refine((value) => !hasDisallowedControl(value));

const rectSchema = z
  .object({
    x: z.number().finite().min(0).max(1),
    y: z.number().finite().min(0).max(1),
    width: z.number().finite().positive().max(1),
    height: z.number().finite().positive().max(1),
  })
  .strict()
  .refine((rect) => rect.x + rect.width <= 1 && rect.y + rect.height <= 1);
const textQuoteSchema = z
  .object({
    exact: boundedText(MAX_SELECTED_TEXT_CODE_POINTS).min(1),
    prefix: boundedText(64),
    suffix: boundedText(64),
  })
  .strict();
const pdfLocatorSchema = z
  .object({
    format: z.literal('pdf'),
    startPage: positiveUint32Schema,
    endPage: positiveUint32Schema,
    rectsByPage: z
      .record(z.string().regex(/^\d+$/u), z.array(rectSchema).max(128))
      .nullable(),
  })
  .strict()
  .superRefine((locator, context) => {
    if (locator.endPage < locator.startPage) {
      context.addIssue({ code: 'custom', message: 'invalid PDF range' });
    }
    if (
      locator.rectsByPage !== null &&
      Object.keys(locator.rectsByPage).some((page) => {
        const number = Number(page);
        return number < locator.startPage || number > locator.endPage;
      })
    ) {
      context.addIssue({
        code: 'custom',
        message: 'invalid PDF rectangle page',
      });
    }
    if (locator.rectsByPage !== null) {
      const pages = Object.values(locator.rectsByPage);
      if (
        pages.length > MAX_PDF_RECT_PAGES ||
        pages.reduce((total, rects) => total + rects.length, 0) >
          MAX_PDF_RECTS_TOTAL
      ) {
        context.addIssue({
          code: 'custom',
          message: 'PDF rectangles exceed bounds',
        });
      }
    }
  });
const epubLocatorSchema = z
  .object({
    format: z.literal('epub'),
    cfi: boundedText(4_096).trim().min(1),
    sectionId: uuidSchema,
  })
  .strict();
const docxLocatorSchema = z
  .object({
    format: z.literal('docx'),
    startBlockId: uuidSchema,
    startOffset: uint32Schema,
    endBlockId: uuidSchema,
    endOffset: uint32Schema,
  })
  .strict();
const documentLocatorSchema = z.discriminatedUnion('format', [
  pdfLocatorSchema,
  epubLocatorSchema,
  docxLocatorSchema,
]);
const textAnchorSchema = z
  .object({
    kind: z.literal('text'),
    selection: z
      .object({
        locator: documentLocatorSchema,
        quote: textQuoteSchema,
        sectionId: uuidSchema.nullable(),
      })
      .strict(),
  })
  .strict();
const regionLocatorSchema = z.discriminatedUnion('format', [
  z.object({ format: z.literal('pdf'), page: positiveUint32Schema }).strict(),
  z
    .object({
      format: z.literal('epub'),
      sectionId: uuidSchema,
      cfi: boundedText(4_096).trim().min(1),
    })
    .strict(),
  z.object({ format: z.literal('docx'), blockId: uuidSchema }).strict(),
]);
const regionAnchorSchema = z
  .object({
    kind: z.literal('region'),
    region: z
      .object({
        locator: regionLocatorSchema,
        rect: rectSchema,
        contentSha256: sha256Schema,
        textFallback: textQuoteSchema.nullable(),
      })
      .strict(),
  })
  .strict();
const contentAnchorSchema = z.discriminatedUnion('kind', [
  textAnchorSchema,
  regionAnchorSchema,
]);

const prepareMetadataSchema = z
  .object({
    bookId: uuidSchema,
    sectionId: uuidSchema,
    providerProfileId: uuidSchema,
    modelId: boundedText(MAX_MODEL_ID_CODE_POINTS).trim().min(1),
    action: z.enum(LEARNING_ACTIONS),
    contentKind: z.enum(LEARNING_CONTENT_KINDS),
    anchor: contentAnchorSchema,
    selectedText: boundedText(MAX_SELECTED_TEXT_CODE_POINTS).nullable(),
    question: boundedText(MAX_QUESTION_CODE_POINTS).nullable(),
    targetLanguage: boundedText(128).nullable(),
  })
  .strict()
  .superRefine((metadata, context) => {
    const kindMatches =
      (metadata.contentKind === 'text_selection' &&
        metadata.anchor.kind === 'text') ||
      (metadata.contentKind !== 'text_selection' &&
        metadata.anchor.kind === 'region');
    if (!kindMatches) {
      context.addIssue({ code: 'custom', message: 'content kind mismatch' });
      return;
    }
    if (metadata.anchor.kind === 'text') {
      if (metadata.selectedText !== metadata.anchor.selection.quote.exact) {
        context.addIssue({ code: 'custom', message: 'selection mismatch' });
      }
    } else {
      const fallback = metadata.anchor.region.textFallback?.exact ?? null;
      if (metadata.contentKind === 'reliable_text_region') {
        if (fallback === null || metadata.selectedText !== fallback) {
          context.addIssue({ code: 'custom', message: 'region text mismatch' });
        }
      } else if (metadata.selectedText !== fallback) {
        context.addIssue({
          code: 'custom',
          message: 'visual fallback mismatch',
        });
      }
    }
    if (metadata.action === 'ask') {
      if (!metadata.question?.trim() || metadata.targetLanguage !== null) {
        context.addIssue({ code: 'custom', message: 'invalid ask metadata' });
      }
    } else if (metadata.action === 'translate') {
      if (!metadata.targetLanguage?.trim() || metadata.question !== null) {
        context.addIssue({
          code: 'custom',
          message: 'invalid translation metadata',
        });
      }
    } else if (metadata.question !== null || metadata.targetLanguage !== null) {
      context.addIssue({
        code: 'custom',
        message: 'unexpected action metadata',
      });
    }
  });

const preparationSummarySchema = z
  .object({
    preparationId: uuidSchema,
    providerDisplayName: boundedText(512).trim().min(1),
    profileDisplayName: boundedText(512).trim().min(1),
    modelDisplayName: boundedText(512).trim().min(1),
    estimatedInputTokens: uint32Schema,
    sourceCount: uint32Schema,
    citationCount: uint32Schema,
    omittedSourceCount: uint32Schema,
    willSendImage: z.boolean(),
    riskFlags: z.array(z.enum(PREPARATION_RISK_FLAGS)).max(2),
    requiresBlockingConfirmation: z.boolean(),
    expiresAt: z.string().regex(ISO_TIMESTAMP),
    actionCategory: z.enum(LEARNING_ACTIONS),
  })
  .strict()
  .superRefine((summary, context) => {
    const risks = new Set(summary.riskFlags);
    if (
      risks.size !== summary.riskFlags.length ||
      risks.has('image_send') !== summary.willSendImage ||
      (summary.requiresBlockingConfirmation && risks.size === 0)
    ) {
      context.addIssue({
        code: 'custom',
        message: 'inconsistent risk summary',
      });
    }
  });

const regionCaptureMetadataSchema = z
  .object({
    preparationId: uuidSchema,
    operationToken: uuidSchema,
    bookId: uuidSchema,
    providerProfileId: uuidSchema,
    modelId: boundedText(MAX_MODEL_ID_CODE_POINTS).trim().min(1),
    anchorContentSha256: sha256Schema,
    captureSha256: sha256Schema,
    schemaVersion: z.literal(1),
    mimeType: z.literal('image/png'),
    width: positiveUint32Schema.max(MAX_CAPTURE_DIMENSION),
    height: positiveUint32Schema.max(MAX_CAPTURE_DIMENSION),
    decodedPixelCount: positiveUint32Schema.max(MAX_CAPTURE_PIXELS),
    encodedByteLength: positiveUint32Schema.max(MAX_CAPTURE_BYTES),
  })
  .strict()
  .refine(
    (metadata) =>
      metadata.width * metadata.height === metadata.decodedPixelCount,
  );
const invalidationSchema = z
  .object({
    reason: z.enum(LEARNING_INVALIDATION_REASONS),
    bookId: uuidSchema.nullable(),
    providerProfileId: uuidSchema.nullable(),
  })
  .strict()
  .refine((request) => {
    switch (request.reason) {
      case 'route_change':
      case 'default_change':
        return request.bookId === null && request.providerProfileId === null;
      case 'book_change':
        return request.bookId !== null && request.providerProfileId === null;
      case 'profile_change':
        return request.bookId === null && request.providerProfileId !== null;
    }
  });
const learningErrorCodeSchema = z.enum(LEARNING_ERROR_CODES);

export interface LearningApi {
  prepare(
    metadata: PrepareLearningRequestMetadata,
  ): Promise<Readonly<PreparationSummary>>;
  authorize(
    preparationId: string,
    decision: LearningAuthorizationDecision,
  ): Promise<string | null>;
  stageRegionCapture(
    metadata: RegionCaptureMetadata,
    bytes: Uint8Array,
  ): Promise<void>;
  discard(preparationId: string): Promise<void>;
  invalidate(request: InvalidateLearningPreparations): Promise<number>;
}

export interface LearningRequestApi {
  start(preparationId: string): Promise<LearningRequestSnapshot>;
  subscribe(
    requestId: string,
    afterSeq: number,
    onEvent: (event: LearningRequestEvent) => void,
  ): Promise<{ snapshot: LearningRequestSnapshot; unsubscribe(): void }>;
  cancel(requestId: string): Promise<void>;
}

export class TauriLearningApi implements LearningApi, LearningRequestApi {
  async prepare(metadata: PrepareLearningRequestMetadata) {
    let request: Readonly<PrepareLearningRequestMetadata>;
    try {
      request = freezePreparationMetadata(
        prepareMetadataSchema.parse(metadata) as PrepareLearningRequestMetadata,
      );
    } catch {
      throw learningError('INVALID_INPUT');
    }
    let response: unknown;
    try {
      response = await invoke<unknown>('prepare_learning_request', {
        metadata: request,
      });
    } catch (error) {
      throw invocationError(error);
    }
    try {
      return freezePreparationSummary(
        preparationSummarySchema.parse(response) as PreparationSummary,
      );
    } catch {
      throw learningError('DATABASE_ERROR');
    }
  }

  async authorize(
    preparationId: string,
    decision: LearningAuthorizationDecision,
  ) {
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
      const response = await invoke<unknown>(
        'authorize_learning_request',
        args,
      );
      return response === null ? null : uuidSchema.parse(response);
    } catch (error) {
      if (isLearningError(error)) throw error;
      throw invocationError(error);
    }
  }

  async stageRegionCapture(
    metadata: RegionCaptureMetadata,
    bytes: Uint8Array,
  ): Promise<void> {
    let safeMetadata: Readonly<RegionCaptureMetadata>;
    try {
      if (!isUint8Array(bytes)) throw new TypeError('invalid bytes');
      safeMetadata = freezeRegionCaptureMetadata(
        regionCaptureMetadataSchema.parse(metadata) as RegionCaptureMetadata,
      );
      if (safeMetadata.encodedByteLength !== bytes.byteLength) {
        throw new TypeError('invalid byte length');
      }
    } catch {
      if (isUint8Array(bytes)) bytes.fill(0);
      throw learningError('INVALID_INPUT');
    }

    let body: Uint8Array;
    try {
      body = Uint8Array.from(bytes);
    } catch {
      bytes.fill(0);
      throw learningError('INVALID_INPUT');
    }
    try {
      const actualHash = await sha256Hex(body);
      if (!constantTimeEqual(actualHash, safeMetadata.captureSha256)) {
        throw learningError('REQUEST_CONFLICT');
      }
      await invoke<void>('stage_region_capture', body, {
        headers: {
          [CAPTURE_METADATA_HEADER]: JSON.stringify(safeMetadata),
        },
      });
    } catch (error) {
      if (isLearningError(error)) throw error;
      throw invocationError(error);
    } finally {
      body.fill(0);
      bytes.fill(0);
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
      await invoke('discard_learning_preparation', { preparationId: id });
    } catch (error) {
      throw invocationError(error);
    }
  }

  async invalidate(request: InvalidateLearningPreparations): Promise<number> {
    let safeRequest: InvalidateLearningPreparations;
    try {
      safeRequest = invalidationSchema.parse(
        request,
      ) as InvalidateLearningPreparations;
    } catch {
      throw learningError('INVALID_INPUT');
    }
    try {
      return uint32Schema.parse(
        await invoke<unknown>('invalidate_learning_preparations', {
          request: safeRequest,
        }),
      );
    } catch (error) {
      if (isLearningError(error)) throw error;
      throw invocationError(error);
    }
  }

  async start(preparationId: string): Promise<LearningRequestSnapshot> {
    try {
      return parseRequestSnapshot(
        await invoke<unknown>('start_learning_request', {
          preparationId: uuidSchema.parse(preparationId),
        }),
      );
    } catch (error) {
      if (isLearningError(error)) throw error;
      throw invocationError(error);
    }
  }

  async subscribe(
    requestId: string,
    afterSeq: number,
    onEvent: (event: LearningRequestEvent) => void,
  ) {
    try {
      const channel = new Channel<unknown>((event) =>
        onEvent(parseRequestEvent(event)),
      );
      const snapshot = parseRequestSnapshot(
        await invoke<unknown>('subscribe_learning_request', {
          requestId: uuidSchema.parse(requestId),
          afterSeq: uint32Schema.parse(afterSeq),
          events: channel,
        }),
      );
      let active = true;
      return {
        snapshot,
        unsubscribe() {
          // Tauri channels have no explicit unsubscribe command. Stop delivery locally;
          // this must never cancel the backend request.
          if (!active) return;
          active = false;
          channel.onmessage = () => {};
        },
      };
    } catch (error) {
      if (isLearningError(error)) throw error;
      throw invocationError(error);
    }
  }

  async cancel(requestId: string): Promise<void> {
    try {
      await invoke('cancel_learning_request', {
        requestId: uuidSchema.parse(requestId),
      });
    } catch (error) {
      if (isLearningError(error)) throw error;
      throw invocationError(error);
    }
  }
}

const learningRequestStatusSchema = z.enum([
  'preparing',
  'streaming',
  'completed',
  'failed',
  'cancelled',
]);
const safeLearningErrorSchema = z.object({ code: z.string().max(64) }).strict();
const learningUsageSchema = z
  .object({
    inputTokens: uint32Schema.nullable(),
    outputTokens: uint32Schema.nullable(),
  })
  .strict();
const learningRequestSnapshotSchema = z
  .object({
    requestId: uuidSchema,
    conversationId: uuidSchema.nullable(),
    status: learningRequestStatusSchema,
    text: z.string().max(MAX_CAPTURE_BYTES),
    usage: learningUsageSchema.nullable(),
    safeError: safeLearningErrorSchema.nullable(),
    lastSeq: uint32Schema,
  })
  .strict()
  .superRefine((snapshot, context) => {
    if (
      (snapshot.status === 'completed' &&
        (!snapshot.conversationId ||
          !snapshot.text.trim() ||
          snapshot.safeError)) ||
      (snapshot.status === 'failed' &&
        (snapshot.conversationId || !snapshot.safeError)) ||
      (['preparing', 'streaming', 'cancelled'].includes(snapshot.status) &&
        (snapshot.conversationId || snapshot.safeError))
    ) {
      context.addIssue({ code: 'custom', message: 'invalid request snapshot' });
    }
  });
const learningRequestEventSchema = z
  .object({
    requestId: uuidSchema,
    seq: positiveUint32Schema,
    event: z.discriminatedUnion('type', [
      z.object({ type: z.literal('preparing') }).strict(),
      z
        .object({
          type: z.literal('text_delta'),
          text: z.string().min(1).max(MAX_CAPTURE_BYTES),
        })
        .strict(),
      z
        .object({
          type: z.literal('usage'),
          inputTokens: uint32Schema.nullable(),
          outputTokens: uint32Schema.nullable(),
        })
        .strict(),
      z
        .object({ type: z.literal('completed'), conversationId: uuidSchema })
        .strict(),
      z
        .object({
          type: z.literal('failed'),
          safeError: safeLearningErrorSchema,
        })
        .strict(),
      z.object({ type: z.literal('cancelled') }).strict(),
    ]),
  })
  .strict();

function parseRequestSnapshot(value: unknown): LearningRequestSnapshot {
  return learningRequestSnapshotSchema.parse(value) as LearningRequestSnapshot;
}

function parseRequestEvent(value: unknown): LearningRequestEvent {
  return learningRequestEventSchema.parse(value) as LearningRequestEvent;
}

function isUint8Array(value: unknown): value is Uint8Array {
  return (
    ArrayBuffer.isView(value) &&
    Object.prototype.toString.call(value) === '[object Uint8Array]'
  );
}

function hasDisallowedControl(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const codeUnit = value.charCodeAt(index);
    if (
      codeUnit <= 0x08 ||
      codeUnit === 0x0b ||
      codeUnit === 0x0c ||
      (codeUnit >= 0x0e && codeUnit <= 0x1f) ||
      codeUnit === 0x7f
    ) {
      return true;
    }
  }
  return false;
}

function invocationError(error: unknown): Readonly<LearningError> {
  const code = z
    .object({ code: learningErrorCodeSchema })
    .passthrough()
    .safeParse(error);
  return learningError(code.success ? code.data.code : 'DATABASE_ERROR');
}

function learningError(code: LearningErrorCode): Readonly<LearningError> {
  return Object.freeze({ code });
}

function isLearningError(error: unknown): error is LearningError {
  return (
    typeof error === 'object' &&
    error !== null &&
    Object.keys(error).length === 1 &&
    learningErrorCodeSchema.safeParse((error as { code?: unknown }).code)
      .success
  );
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const copy = Uint8Array.from(bytes);
  try {
    const digest = await crypto.subtle.digest('SHA-256', copy.buffer);
    return [...new Uint8Array(digest)]
      .map((value) => value.toString(16).padStart(2, '0'))
      .join('');
  } finally {
    copy.fill(0);
  }
}

function constantTimeEqual(left: string, right: string): boolean {
  if (left.length !== right.length) return false;
  let difference = 0;
  for (let index = 0; index < left.length; index += 1) {
    difference |= left.charCodeAt(index) ^ right.charCodeAt(index);
  }
  return difference === 0;
}
