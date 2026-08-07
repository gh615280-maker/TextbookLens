import { invoke } from '@tauri-apps/api/core';
import { z } from 'zod';
import { toUserError } from '../../lib/errors';

const summarySchema = z
  .object({
    id: z.string().uuid(),
    bookId: z.string().uuid(),
    status: z.enum([
      'queued',
      'uploading',
      'processing',
      'downloading',
      'indexing',
      'ready',
      'cancelling',
      'cancelled',
      'failed',
    ]),
    contentChars: z.number().int().nonnegative(),
    chunkCount: z.number().int().nonnegative(),
    reused: z.boolean(),
  })
  .strict();
export type ExtractionSummary = z.infer<typeof summarySchema>;

export class TauriExtractionApi {
  async prepare(bookId: string, providerProfileId: string) {
    try {
      return summarySchema.parse(
        await invoke('prepare_book_extraction', { bookId, providerProfileId }),
      );
    } catch (error) {
      throw toUserError(error);
    }
  }
  async get(bookId: string) {
    try {
      const value = await invoke('get_book_extraction', { bookId });
      return value === null ? null : summarySchema.parse(value);
    } catch (error) {
      throw toUserError(error);
    }
  }
  async cancel(bookId: string) {
    try {
      await invoke('cancel_book_extraction', { bookId });
    } catch (error) {
      throw toUserError(error);
    }
  }
}
