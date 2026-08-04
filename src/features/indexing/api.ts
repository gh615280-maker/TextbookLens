import { invoke } from '@tauri-apps/api/core';
import { z } from 'zod';

import type {
  IndexFailureCode,
  IndexPageStatus,
  IndexQualityReason,
} from '../../lib/generated/indexing';
import { toUserError } from '../../lib/errors';
import type { RenderedPdfPageDto } from './indexing-contract';

const uuidSchema = z.uuid();
const sha256Schema = z.string().regex(/^[0-9a-f]{64}$/u);
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
  retryPage(pageId: string, attemptId: string): Promise<string>;
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

  async retryPage(pageId: string, attemptId: string): Promise<string> {
    return invokeSafeUuid('retry_index_page', {
      pageId: uuidSchema.parse(pageId),
      attemptId: uuidSchema.parse(attemptId),
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
