import { invoke } from '@tauri-apps/api/core';

import { toUserError } from '../../lib/errors';
import type { BookSummary } from '../../lib/generated/book';
import { parseBookSummaries } from '../../lib/ipc';

export interface LibraryApi {
  listBooks(): Promise<BookSummary[]>;
  deleteFailedImport(bookId: string): Promise<void>;
}

export class TauriLibraryApi implements LibraryApi {
  async listBooks(): Promise<BookSummary[]> {
    try {
      return parseBookSummaries(await invoke<unknown>('list_books'));
    } catch (error) {
      throw toUserError(error);
    }
  }

  async deleteFailedImport(bookId: string): Promise<void> {
    try {
      await invoke('delete_failed_import', { bookId });
    } catch (error) {
      throw toUserError(error);
    }
  }
}
