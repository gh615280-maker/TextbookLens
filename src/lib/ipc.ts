import { Channel, invoke } from '@tauri-apps/api/core';
import { z } from 'zod';

import type { BookSummary } from './generated/book';
import type { NormalizedSectionInput } from './generated/document';
import type {
  BeginImportOutcome,
  BeginImportRequest,
  ImportErrorCode,
  ImportEvent,
  ImportStage,
  ParsedBookMetadata,
} from '../features/import/parser-contract';

const bookFormatSchema = z.enum(['pdf', 'epub', 'docx']);
const importStatusSchema = z.enum([
  'queued',
  'copying',
  'parsing',
  'indexing',
  'ready',
  'failed',
]);
const importStageSchema = z.enum(['copying', 'parsing', 'indexing']);
const indexAggregateStatusSchema = z.enum([
  'not_required',
  'ready',
  'partial',
  'needs_review',
  'failed',
]);
const indexPageCountSchema = z.int().nonnegative().max(1_000_000);
const indexAggregateSchema = z
  .object({
    status: indexAggregateStatusSchema,
    totalPages: indexPageCountSchema,
    indexedPages: indexPageCountSchema,
    reviewPages: indexPageCountSchema,
    failedPages: indexPageCountSchema,
  })
  .strict()
  .superRefine((aggregate, context) => {
    const accounted =
      aggregate.indexedPages + aggregate.reviewPages + aggregate.failedPages;
    if (accounted > aggregate.totalPages) {
      context.addIssue({
        code: 'custom',
        message: 'Index aggregate counts exceed totalPages.',
      });
    }

    switch (aggregate.status) {
      case 'not_required':
        if (aggregate.totalPages !== 0 || accounted !== 0) {
          context.addIssue({
            code: 'custom',
            message: 'A not-required aggregate has no indexed pages.',
          });
        }
        break;
      case 'ready':
        if (
          aggregate.totalPages === 0 ||
          aggregate.reviewPages !== 0 ||
          aggregate.failedPages !== 0
        ) {
          context.addIssue({
            code: 'custom',
            message: 'A ready aggregate has no review or failed pages.',
          });
        }
        break;
      case 'partial':
        if (
          aggregate.totalPages === 0 ||
          (aggregate.failedPages === 0 && accounted === aggregate.totalPages)
        ) {
          context.addIssue({
            code: 'custom',
            message:
              'A partial aggregate must retain unresolved or failed work.',
          });
        }
        break;
      case 'needs_review':
        if (aggregate.reviewPages === 0 || aggregate.failedPages !== 0) {
          context.addIssue({
            code: 'custom',
            message:
              'A review aggregate must have review pages and no failures.',
          });
        }
        break;
      case 'failed':
        if (
          aggregate.failedPages === 0 ||
          aggregate.indexedPages !== 0 ||
          aggregate.reviewPages !== 0 ||
          aggregate.failedPages !== aggregate.totalPages
        ) {
          context.addIssue({
            code: 'custom',
            message: 'A failed aggregate has only failed pages.',
          });
        }
        break;
    }
  });
const bookSummarySchema = z
  .object({
    id: z.uuid(),
    title: z.string(),
    originalFilename: z.string().min(1),
    author: z.string().nullable(),
    language: z.string().nullable(),
    format: bookFormatSchema,
    importStatus: importStatusSchema,
    importErrorCode: z.string().nullable(),
    importErrorMessage: z.string().nullable(),
    importErrorStage: importStageSchema.nullable(),
    readingProgress: z.number().min(0).max(1),
    indexAggregate: indexAggregateSchema,
    createdAt: z.string(),
    updatedAt: z.string(),
    lastOpenedAt: z.string().nullable(),
  })
  .strict();
const beginImportOutcomeSchema = z.discriminatedUnion('outcome', [
  z.object({ outcome: z.literal('created'), book: bookSummarySchema }).strict(),
  z
    .object({ outcome: z.literal('duplicate'), book: bookSummarySchema })
    .strict(),
]);
const parsedBookMetadataSchema = z
  .object({
    title: z.string().trim().min(1),
    author: z.string().nullable(),
    language: z.string().nullable(),
  })
  .strict();
const importEventSchema = z
  .object({
    stage: importStageSchema,
    completed: z.number().nonnegative(),
    total: z.number().nonnegative(),
    messageKey: z.string().min(1),
  })
  .strict();

export interface ImportIpc {
  beginImport(
    sourcePath: string,
    onProgress: (event: ImportEvent) => void,
  ): Promise<BeginImportOutcome>;
  readBookSource(bookId: string): Promise<Uint8Array | ArrayBuffer>;
  beginParse(bookId: string, metadata: ParsedBookMetadata): Promise<void>;
  appendParsedSections(
    bookId: string,
    sections: NormalizedSectionInput[],
  ): Promise<void>;
  writeDerivedText(
    bookId: string,
    name: 'document.html',
    content: string,
  ): Promise<void>;
  finalizeImport(
    bookId: string,
    onProgress: (event: ImportEvent) => void,
  ): Promise<BookSummary>;
  cancelImport(bookId: string): Promise<void>;
  markImportFailed(
    bookId: string,
    stage: ImportStage,
    code: ImportErrorCode,
  ): Promise<void>;
  retryImport(
    bookId: string,
    replacementSourcePath: string | null,
  ): Promise<BeginImportOutcome>;
}

export class TauriImportIpc implements ImportIpc {
  async beginImport(
    sourcePath: string,
    onProgress: (event: ImportEvent) => void,
  ): Promise<BeginImportOutcome> {
    const request: BeginImportRequest = { sourcePath };
    const outcome = await invoke<unknown>('begin_import', {
      request,
      progress: progressChannel(onProgress),
    });
    return beginImportOutcomeSchema.parse(outcome) as BeginImportOutcome;
  }

  async readBookSource(bookId: string): Promise<Uint8Array> {
    const result = await invoke<ArrayBuffer | Uint8Array | number[]>(
      'read_book_source',
      { bookId },
    );
    if (result instanceof Uint8Array) return result;
    if (result instanceof ArrayBuffer) return new Uint8Array(result);
    if (Array.isArray(result) && result.every(isByte)) {
      return Uint8Array.from(result);
    }
    throw new TypeError('read_book_source returned an invalid binary payload');
  }

  async beginParse(
    bookId: string,
    metadata: ParsedBookMetadata,
  ): Promise<void> {
    await invoke('begin_parse', {
      bookId,
      metadata: parsedBookMetadataSchema.parse(metadata),
    });
  }

  async appendParsedSections(
    bookId: string,
    sections: NormalizedSectionInput[],
  ): Promise<void> {
    await invoke('append_parsed_sections', { bookId, sections });
  }

  async writeDerivedText(
    bookId: string,
    name: 'document.html',
    content: string,
  ): Promise<void> {
    await invoke('write_derived_text', { bookId, name, content });
  }

  async finalizeImport(
    bookId: string,
    onProgress: (event: ImportEvent) => void,
  ): Promise<BookSummary> {
    const result = await invoke<unknown>('finalize_import', {
      bookId,
      progress: progressChannel(onProgress),
    });
    return bookSummarySchema.parse(result) as BookSummary;
  }

  async cancelImport(bookId: string): Promise<void> {
    await invoke('cancel_import', { bookId });
  }

  async markImportFailed(
    bookId: string,
    stage: ImportStage,
    code: ImportErrorCode,
  ): Promise<void> {
    await invoke('mark_import_failed', {
      bookId,
      stage: importStageSchema.parse(stage),
      code,
    });
  }

  async retryImport(
    bookId: string,
    replacementSourcePath: string | null,
  ): Promise<BeginImportOutcome> {
    const outcome = await invoke<unknown>('retry_import', {
      bookId,
      replacementSourcePath,
    });
    return beginImportOutcomeSchema.parse(outcome) as BeginImportOutcome;
  }
}

export function parseBookSummary(value: unknown): BookSummary {
  return bookSummarySchema.parse(value) as BookSummary;
}

export function parseBookSummaries(value: unknown): BookSummary[] {
  return z.array(bookSummarySchema).parse(value) as BookSummary[];
}

export function toOwnedArrayBuffer(
  binary: Uint8Array | ArrayBuffer,
): ArrayBuffer {
  const view = binary instanceof Uint8Array ? binary : new Uint8Array(binary);
  return Uint8Array.from(view).buffer;
}

function progressChannel(
  onProgress: (event: ImportEvent) => void,
): Channel<unknown> {
  return new Channel((event) => onProgress(importEventSchema.parse(event)));
}

function isByte(value: number): boolean {
  return Number.isInteger(value) && value >= 0 && value <= 255;
}
