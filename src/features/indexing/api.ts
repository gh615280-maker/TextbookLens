import { invoke } from '@tauri-apps/api/core';
import { z } from 'zod';

import type {
  IndexPageReviewDto,
  IndexFailureCode,
  IndexPageStatus,
  IndexQualityReason,
  IndexRunAggregateDto,
} from '../../lib/generated/indexing';
import type {
  IndexCorrectionReviewDto,
  IndexCorrectionValueKind,
} from '../../lib/generated/provenance';
import { toUserError } from '../../lib/errors';
import type { RenderedPdfPageDto } from './indexing-contract';

const uuidSchema = z.uuid();
const sha256Schema = z.string().regex(/^[0-9a-f]{64}$/u);
const databaseTimestampSchema = z
  .string()
  .regex(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/u)
  .refine((value) => Number.isFinite(Date.parse(value)));
const statusSchema = z.enum([
  'not_required',
  'queued',
  'rendering',
  'sending',
  'parsing',
  'validating',
  'indexed',
  'needs_review',
  'failed',
  'cancelled',
]);
const safeErrorCodeSchema = z
  .enum([
    'INDEX_RENDER_FAILED',
    'INDEX_PROVIDER_FAILED',
    'INDEX_RESPONSE_INVALID',
    'INDEX_VALIDATION_FAILED',
    'INDEX_ATTEMPT_INTERRUPTED',
    'INDEX_SCRATCH_MISSING',
    'INDEX_ATTEMPT_EXPIRED',
  ])
  .nullable();
const renderLimitsSchema = z
  .object({
    maxDimension: z.number().int().positive().max(4096),
    maxDecodedPixels: z.number().int().positive().max(8_847_360),
    maxEncodedBytes: z
      .number()
      .int()
      .positive()
      .max(4 * 1024 * 1024),
    maxTotalEncodedBytes: z
      .number()
      .int()
      .positive()
      .max(12 * 1024 * 1024),
  })
  .strict();
const renderClaimSchema = z
  .object({
    runId: uuidSchema,
    bookId: uuidSchema,
    pageId: uuidSchema,
    pageNumber: z.number().int().positive().max(1_000_000),
    attemptId: uuidSchema,
    limits: renderLimitsSchema,
  })
  .strict();
const renderClaimBatchSchema = z
  .object({ claims: z.array(renderClaimSchema).max(2) })
  .strict()
  .superRefine((batch, context) => {
    const first = batch.claims[0];
    if (!first) return;
    const pageIds = new Set<string>();
    for (const claim of batch.claims) {
      if (
        claim.runId !== first.runId ||
        claim.bookId !== first.bookId ||
        pageIds.has(claim.pageId)
      ) {
        context.addIssue({ code: 'custom', message: 'unsafe render batch' });
        return;
      }
      pageIds.add(claim.pageId);
    }
  });
const indexingEventSchema = z
  .object({
    runId: uuidSchema,
    pageId: uuidSchema,
    status: statusSchema,
    safeErrorCode: safeErrorCodeSchema,
  })
  .strict();
const correctionSchema = z
  .object({
    id: uuidSchema,
    targetBlockId: uuidSchema,
    region: z
      .object({
        x: z.number(),
        y: z.number(),
        width: z.number(),
        height: z.number(),
      })
      .strict()
      .nullable(),
    valueKind: z.enum(['text', 'latex']),
    originalValue: z.string(),
    correctedValue: z.string(),
    conflictState: z.enum(['active', 'conflict']),
    revision: z.number().int().nonnegative(),
    updatedAt: z.string(),
  })
  .strict();
const reviewSchema = z
  .object({
    id: uuidSchema,
    runId: uuidSchema,
    bookId: uuidSchema,
    pageNumber: z.number().int().positive(),
    qualityReason: z.enum([
      'reliable_text',
      'no_text',
      'very_low_text_coverage',
      'high_replacement_or_control_ratio',
      'extreme_duplicate_glyphs',
      'layout_contradiction',
    ]),
    status: statusSchema,
    reviewReason: z
      .enum([
        'incomplete_content',
        'invalid_bounds',
        'severe_overlap',
        'text_contradiction',
        'malformed_table',
        'malformed_latex',
      ])
      .nullable(),
    safeError: z
      .object({
        code: safeErrorCodeSchema.unwrap(),
        message: z.string(),
        retryable: z.boolean(),
      })
      .strict()
      .nullable(),
    contentVersion: z.number().int().nonnegative(),
    blocks: z.array(
      z
        .object({
          id: uuidSchema,
          ordinal: z.number().int().nonnegative(),
          kind: z.enum([
            'title',
            'paragraph',
            'list',
            'table',
            'caption',
            'formula',
            'figure',
            'transcript',
          ]),
          source: z.enum([
            'local_text',
            'ai_transcribed',
            'ai_description',
            'user_corrected',
          ]),
          plainText: z.string().nullable(),
          latex: z.string().nullable(),
          tableCells: z
            .array(
              z
                .object({
                  row: z.number().int().nonnegative(),
                  column: z.number().int().nonnegative(),
                  rowSpan: z.number().int().positive(),
                  columnSpan: z.number().int().positive(),
                  text: z.string(),
                })
                .strict(),
            )
            .nullable(),
          visualDescription: z.string().nullable(),
          bounds: z
            .object({
              x: z.number(),
              y: z.number(),
              width: z.number(),
              height: z.number(),
            })
            .strict()
            .nullable(),
          contentVersion: z.number().int().positive(),
        })
        .strict(),
    ),
    corrections: z.array(correctionSchema),
    updatedAt: databaseTimestampSchema,
  })
  .strict();
const aggregateSchema = z
  .object({
    runId: uuidSchema,
    bookId: uuidSchema,
    controlStatus: z.enum([
      'running',
      'paused',
      'cancelling',
      'cancelled',
      'completed',
    ]),
    aggregateStatus: z.enum(['ready', 'partial', 'needs_review', 'failed']),
    pages: z
      .object({
        total: z.number().int().nonnegative(),
        notRequired: z.number().int().nonnegative(),
        queued: z.number().int().nonnegative(),
        rendering: z.number().int().nonnegative(),
        sending: z.number().int().nonnegative(),
        parsing: z.number().int().nonnegative(),
        validating: z.number().int().nonnegative(),
        indexed: z.number().int().nonnegative(),
        needsReview: z.number().int().nonnegative(),
        failed: z.number().int().nonnegative(),
        cancelled: z.number().int().nonnegative(),
      })
      .strict(),
    updatedAt: z.string(),
  })
  .strict();

export interface IndexPageSeed {
  pageNumber: number;
  qualityReason: IndexQualityReason;
  localTextSha256: string | null;
}

export interface ConfirmIndexOperationRequest {
  runId: string | null;
  bookId: string;
  sourceSha256: string;
  providerProfileId: string;
  pages: IndexPageSeed[];
}

export interface RenderLimits {
  maxDimension: number;
  maxDecodedPixels: number;
  maxEncodedBytes: number;
  maxTotalEncodedBytes: number;
}

export interface RenderClaim {
  runId: string;
  bookId: string;
  pageId: string;
  pageNumber: number;
  attemptId: string;
  limits: RenderLimits;
}

export interface RenderClaimBatch {
  claims: RenderClaim[];
}

export interface IndexingEvent {
  runId: string;
  pageId: string;
  status: IndexPageStatus;
  safeErrorCode: IndexFailureCode | null;
}

export interface RenderedPageSubmission {
  runId: string;
  bookId: string;
  pageId: string;
  pageNumber: number;
  attemptId: string;
  schemaVersion: 1;
  mimeType: 'image/png';
  width: number;
  height: number;
  decodedPixelCount: number;
  encodedByteLength: number;
  sha256: string;
  bytes: Uint8Array;
}

export interface SaveIndexCorrectionRequest {
  bookId: string;
  pageId: string;
  targetBlockId: string;
  targetContentVersion: number;
  valueKind: IndexCorrectionValueKind;
  originalValueSha256: string;
  correctedValue: string;
  expectedRevision: number;
}

export interface ResolveIndexCorrectionConflictRequest {
  bookId: string;
  pageId: string;
  correctionId: string;
  targetContentVersion: number;
  currentValueSha256: string | null;
  expectedRevision: number;
  decision: 'keep' | 'accept' | 'compare';
  comparedCorrectedValue: string | null;
}

export interface DeleteIndexCorrectionRequest {
  bookId: string;
  pageId: string;
  correctionId: string;
  targetContentVersion: number;
  currentValueSha256: string | null;
  expectedRevision: number;
}

export interface IndexingApi {
  confirmOperation(request: ConfirmIndexOperationRequest): Promise<string>;
  createRun(
    operationToken: string,
    request: ConfirmIndexOperationRequest,
  ): Promise<string>;
  authorizeRun(
    runId: string,
    operationToken: string,
    request: ConfirmIndexOperationRequest,
  ): Promise<void>;
  claimRenderBatch(): Promise<RenderClaimBatch>;
  readClaimedSource(pageId: string, attemptId: string): Promise<Uint8Array>;
  submitRenderedBatch(
    submissions: RenderedPageSubmission[],
  ): Promise<IndexingEvent[]>;
  reportRenderFailure(
    pageId: string,
    attemptId: string,
  ): Promise<IndexingEvent>;
  pauseRun(runId: string): Promise<void>;
  resumeRun(runId: string): Promise<void>;
  cancelRun(runId: string): Promise<void>;
  retryPage(pageId: string, expectedUpdatedAt: string): Promise<unknown>;
  getRunAggregate(runId: string): Promise<IndexRunAggregateDto>;
  findCurrentRunForBook?(bookId: string): Promise<IndexRunAggregateDto | null>;
  listPageReviews(runId: string): Promise<IndexPageReviewDto[]>;
  getPageReview(pageId: string): Promise<IndexPageReviewDto>;
  listPageCorrections(pageId: string): Promise<IndexCorrectionReviewDto[]>;
  saveCorrection(
    request: SaveIndexCorrectionRequest,
  ): Promise<IndexCorrectionReviewDto>;
  resolveCorrectionConflict(
    request: ResolveIndexCorrectionConflictRequest,
  ): Promise<IndexCorrectionReviewDto | null>;
  deleteCorrection(request: DeleteIndexCorrectionRequest): Promise<void>;
}

export class TauriIndexingApi implements IndexingApi {
  async confirmOperation(request: ConfirmIndexOperationRequest) {
    return invokeSafeUuid('confirm_index_operation', {
      request: parseConfirmation(request),
    });
  }

  async createRun(
    operationToken: string,
    request: ConfirmIndexOperationRequest,
  ) {
    return invokeSafeUuid('create_index_run', {
      operationToken: uuidSchema.parse(operationToken),
      request: parseConfirmation(request),
    });
  }

  async authorizeRun(
    runId: string,
    operationToken: string,
    request: ConfirmIndexOperationRequest,
  ): Promise<void> {
    await invokeSafe('authorize_index_run', {
      runId: uuidSchema.parse(runId),
      operationToken: uuidSchema.parse(operationToken),
      request: parseConfirmation(request),
    });
  }

  async claimRenderBatch(): Promise<RenderClaimBatch> {
    const result = await invokeSafe('claim_index_render_batch');
    return renderClaimBatchSchema.parse(result) as RenderClaimBatch;
  }

  async readClaimedSource(
    pageId: string,
    attemptId: string,
  ): Promise<Uint8Array> {
    const result = await invokeSafe('read_claimed_index_source', {
      pageId: uuidSchema.parse(pageId),
      attemptId: uuidSchema.parse(attemptId),
    });
    if (result instanceof Uint8Array) return Uint8Array.from(result);
    if (result instanceof ArrayBuffer) return new Uint8Array(result.slice(0));
    if (Array.isArray(result) && result.every(isByte)) {
      return Uint8Array.from(result as number[]);
    }
    throw new TypeError('read_claimed_index_source returned invalid bytes');
  }

  async submitRenderedBatch(
    submissions: RenderedPageSubmission[],
  ): Promise<IndexingEvent[]> {
    if (submissions.length === 0 || submissions.length > 2) {
      throw new TypeError('invalid rendered batch size');
    }
    for (const submission of submissions) validateSubmission(submission);
    const first = submissions[0];
    if (
      submissions.some(
        (submission) =>
          submission.runId !== first.runId ||
          submission.bookId !== first.bookId,
      ) ||
      new Set(submissions.map((submission) => submission.pageId)).size !==
        submissions.length
    ) {
      throw new TypeError('unsafe rendered batch');
    }
    const totalBytes = submissions.reduce(
      (total, submission) => total + submission.encodedByteLength,
      0,
    );
    if (totalBytes <= 0 || totalBytes > 12 * 1024 * 1024) {
      throw new TypeError('rendered batch exceeds total byte limit');
    }
    const body = new Uint8Array(totalBytes);
    let byteOffset = 0;
    const metadata = submissions.map((submission) => {
      body.set(submission.bytes, byteOffset);
      const capture = {
        runId: submission.runId,
        bookId: submission.bookId,
        pageId: submission.pageId,
        pageNumber: submission.pageNumber,
        attemptId: submission.attemptId,
        schemaVersion: submission.schemaVersion,
        mimeType: submission.mimeType,
        width: submission.width,
        height: submission.height,
        decodedPixelCount: submission.decodedPixelCount,
        encodedByteLength: submission.encodedByteLength,
        sha256: submission.sha256,
        byteOffset,
      };
      byteOffset += submission.encodedByteLength;
      return capture;
    });
    try {
      const result = await invoke<unknown>('submit_index_render_batch', body, {
        headers: {
          'x-textbooklens-index-captures': JSON.stringify(metadata),
        },
      }).catch((error: unknown) => {
        throw toUserError(error);
      });
      return z
        .array(indexingEventSchema)
        .max(2)
        .parse(result) as IndexingEvent[];
    } finally {
      body.fill(0);
    }
  }

  async reportRenderFailure(pageId: string, attemptId: string) {
    const result = await invokeSafe('report_index_render_failure', {
      pageId: uuidSchema.parse(pageId),
      attemptId: uuidSchema.parse(attemptId),
    });
    return indexingEventSchema.parse(result) as IndexingEvent;
  }

  async pauseRun(runId: string): Promise<void> {
    await invokeSafe('pause_index_run', { runId: uuidSchema.parse(runId) });
  }

  async resumeRun(runId: string): Promise<void> {
    await invokeSafe('resume_index_run', { runId: uuidSchema.parse(runId) });
  }

  async cancelRun(runId: string): Promise<void> {
    await invokeSafe('cancel_index_run', { runId: uuidSchema.parse(runId) });
  }

  async retryPage(pageId: string, expectedUpdatedAt: string): Promise<void> {
    await invokeSafe('retry_index_page', {
      pageId: uuidSchema.parse(pageId),
      expectedUpdatedAt: databaseTimestampSchema.parse(expectedUpdatedAt),
    });
  }

  async getRunAggregate(runId: string): Promise<IndexRunAggregateDto> {
    return aggregateSchema.parse(
      await invokeSafe('get_index_run_aggregate', {
        runId: uuidSchema.parse(runId),
      }),
    ) as IndexRunAggregateDto;
  }

  async findCurrentRunForBook(
    bookId: string,
  ): Promise<IndexRunAggregateDto | null> {
    const result = await invokeSafe('find_current_index_run_for_book', {
      bookId: uuidSchema.parse(bookId),
    });
    return result === null
      ? null
      : (aggregateSchema.parse(result) as IndexRunAggregateDto);
  }

  async listPageReviews(runId: string): Promise<IndexPageReviewDto[]> {
    return z.array(reviewSchema).parse(
      await invokeSafe('list_index_page_reviews', {
        runId: uuidSchema.parse(runId),
      }),
    ) as IndexPageReviewDto[];
  }

  async getPageReview(pageId: string): Promise<IndexPageReviewDto> {
    return reviewSchema.parse(
      await invokeSafe('get_index_page_review', {
        pageId: uuidSchema.parse(pageId),
      }),
    ) as IndexPageReviewDto;
  }

  async listPageCorrections(
    pageId: string,
  ): Promise<IndexCorrectionReviewDto[]> {
    return z.array(correctionSchema).parse(
      await invokeSafe('list_index_page_corrections', {
        pageId: uuidSchema.parse(pageId),
      }),
    ) as IndexCorrectionReviewDto[];
  }

  async saveCorrection(
    request: SaveIndexCorrectionRequest,
  ): Promise<IndexCorrectionReviewDto> {
    return correctionSchema.parse(
      await invokeSafe('save_index_correction', {
        request: parseCorrectionRequest(request),
      }),
    ) as IndexCorrectionReviewDto;
  }

  async resolveCorrectionConflict(
    request: ResolveIndexCorrectionConflictRequest,
  ): Promise<IndexCorrectionReviewDto | null> {
    const result = await invokeSafe('resolve_index_correction_conflict', {
      request: parseResolveCorrectionRequest(request),
    });
    return result === null
      ? null
      : (correctionSchema.parse(result) as IndexCorrectionReviewDto);
  }

  async deleteCorrection(request: DeleteIndexCorrectionRequest): Promise<void> {
    await invokeSafe('delete_index_correction', {
      request: parseDeleteCorrectionRequest(request),
    });
  }
}

const qualityReasonSchema = z.enum([
  'reliable_text',
  'no_text',
  'very_low_text_coverage',
  'high_replacement_or_control_ratio',
  'extreme_duplicate_glyphs',
  'layout_contradiction',
]);
const confirmationSchema = z
  .object({
    runId: uuidSchema.nullable(),
    bookId: uuidSchema,
    sourceSha256: sha256Schema,
    providerProfileId: uuidSchema,
    pages: z
      .array(
        z
          .object({
            pageNumber: z.number().int().positive().max(1_000_000),
            qualityReason: qualityReasonSchema,
            localTextSha256: sha256Schema.nullable(),
          })
          .strict(),
      )
      .min(1)
      .max(10_000),
  })
  .strict()
  .superRefine((request, context) => {
    const pages = new Set<number>();
    for (const page of request.pages) {
      if (pages.has(page.pageNumber)) {
        context.addIssue({ code: 'custom', message: 'duplicate page' });
        return;
      }
      pages.add(page.pageNumber);
    }
  });

function parseConfirmation(
  request: ConfirmIndexOperationRequest,
): ConfirmIndexOperationRequest {
  return confirmationSchema.parse(request) as ConfirmIndexOperationRequest;
}

const correctionRequestSchema = z
  .object({
    bookId: uuidSchema,
    pageId: uuidSchema,
    targetBlockId: uuidSchema,
    targetContentVersion: z.number().int().positive(),
    valueKind: z.enum(['text', 'latex']),
    originalValueSha256: sha256Schema,
    correctedValue: z.string(),
    expectedRevision: z.number().int().nonnegative(),
  })
  .strict();
const resolveCorrectionRequestSchema = z
  .object({
    bookId: uuidSchema,
    pageId: uuidSchema,
    correctionId: uuidSchema,
    targetContentVersion: z.number().int().positive(),
    currentValueSha256: sha256Schema.nullable(),
    expectedRevision: z.number().int().positive(),
    decision: z.enum(['keep', 'accept', 'compare']),
    comparedCorrectedValue: z.string().nullable(),
  })
  .strict();
const deleteCorrectionRequestSchema = z
  .object({
    bookId: uuidSchema,
    pageId: uuidSchema,
    correctionId: uuidSchema,
    targetContentVersion: z.number().int().positive(),
    currentValueSha256: sha256Schema.nullable(),
    expectedRevision: z.number().int().positive(),
  })
  .strict();

function parseCorrectionRequest(
  request: SaveIndexCorrectionRequest,
): SaveIndexCorrectionRequest {
  return correctionRequestSchema.parse(request) as SaveIndexCorrectionRequest;
}

function parseResolveCorrectionRequest(
  request: ResolveIndexCorrectionConflictRequest,
): ResolveIndexCorrectionConflictRequest {
  return resolveCorrectionRequestSchema.parse(
    request,
  ) as ResolveIndexCorrectionConflictRequest;
}

function parseDeleteCorrectionRequest(
  request: DeleteIndexCorrectionRequest,
): DeleteIndexCorrectionRequest {
  return deleteCorrectionRequestSchema.parse(
    request,
  ) as DeleteIndexCorrectionRequest;
}

function validateSubmission(submission: RenderedPageSubmission): void {
  uuidSchema.parse(submission.runId);
  uuidSchema.parse(submission.bookId);
  uuidSchema.parse(submission.pageId);
  uuidSchema.parse(submission.attemptId);
  sha256Schema.parse(submission.sha256);
  if (
    submission.schemaVersion !== 1 ||
    submission.mimeType !== 'image/png' ||
    !(submission.bytes instanceof Uint8Array) ||
    !Number.isInteger(submission.pageNumber) ||
    submission.pageNumber <= 0 ||
    !Number.isInteger(submission.width) ||
    !Number.isInteger(submission.height) ||
    submission.width <= 0 ||
    submission.height <= 0 ||
    submission.width > 4096 ||
    submission.height > 4096 ||
    submission.decodedPixelCount !== submission.width * submission.height ||
    submission.decodedPixelCount > 8_847_360 ||
    submission.encodedByteLength !== submission.bytes.byteLength ||
    submission.encodedByteLength <= 0 ||
    submission.encodedByteLength > 4 * 1024 * 1024
  ) {
    throw new TypeError('invalid rendered page submission');
  }
}

async function invokeSafe(
  command: string,
  args?: Record<string, unknown>,
): Promise<unknown> {
  try {
    return await invoke<unknown>(command, args);
  } catch (error) {
    throw toUserError(error);
  }
}

async function invokeSafeUuid(
  command: string,
  args: Record<string, unknown>,
): Promise<string> {
  return uuidSchema.parse(await invokeSafe(command, args));
}

function isByte(value: unknown): value is number {
  return (
    typeof value === 'number' &&
    Number.isInteger(value) &&
    value >= 0 &&
    value <= 255
  );
}

export function toSubmission(
  claim: RenderClaim,
  rendered: RenderedPdfPageDto,
): RenderedPageSubmission {
  return {
    runId: claim.runId,
    bookId: claim.bookId,
    pageId: claim.pageId,
    pageNumber: claim.pageNumber,
    attemptId: claim.attemptId,
    schemaVersion: rendered.schemaVersion,
    mimeType: rendered.mimeType,
    width: rendered.width,
    height: rendered.height,
    decodedPixelCount: rendered.decodedPixelCount,
    encodedByteLength: rendered.encodedByteLength,
    sha256: rendered.sha256,
    bytes: rendered.bytes,
  };
}
