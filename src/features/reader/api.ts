import { invoke } from '@tauri-apps/api/core';

import { toUserError } from '../../lib/errors';
import type { BookSummary } from '../../lib/generated/book';
import type { DocumentLocator } from '../../lib/generated/document';

export interface ReaderSettings {
  fontScale: number;
  lineHeight: number;
  readerWidth: number;
  pdfZoom: number;
}

export interface ReaderBootstrap {
  book: BookSummary;
  lastLocator: DocumentLocator | null;
}

export interface ReaderApi {
  getReaderBootstrap(bookId: string): Promise<ReaderBootstrap>;
  readBookSource(bookId: string): Promise<Uint8Array | ArrayBuffer>;
  readDerivedText(bookId: string, name: 'document.html'): Promise<string>;
  getReaderSettings(): Promise<ReaderSettings>;
  updateReaderSettings(settings: ReaderSettings): Promise<ReaderSettings>;
  saveReadingProgress(bookId: string, progress: number, locator: DocumentLocator): Promise<void>;
}

export class TauriReaderApi implements ReaderApi {
  async getReaderBootstrap(bookId: string): Promise<ReaderBootstrap> {
    try {
      return await invoke<ReaderBootstrap>('get_reader_bootstrap', { bookId });
    } catch (error) {
      throw toUserError(error);
    }
  }

  async readBookSource(bookId: string): Promise<Uint8Array> {
    try {
      const source = await invoke<unknown>('read_book_source', { bookId });
      if (source instanceof Uint8Array) return source;
      if (source instanceof ArrayBuffer) return new Uint8Array(source);
      throw new TypeError('read_book_source returned an invalid binary payload');
    } catch (error) {
      throw toUserError(error);
    }
  }

  async readDerivedText(bookId: string, name: 'document.html'): Promise<string> {
    try {
      return await invoke<string>('read_derived_text', { bookId, name });
    } catch (error) {
      throw toUserError(error);
    }
  }

  async getReaderSettings(): Promise<ReaderSettings> {
    try {
      return await invoke<ReaderSettings>('get_reader_settings');
    } catch (error) {
      throw toUserError(error);
    }
  }

  async updateReaderSettings(settings: ReaderSettings): Promise<ReaderSettings> {
    try {
      return await invoke<ReaderSettings>('update_reader_settings', { settings });
    } catch (error) {
      throw toUserError(error);
    }
  }

  async saveReadingProgress(bookId: string, progress: number, locator: DocumentLocator): Promise<void> {
    try {
      await invoke('save_reading_progress', { bookId, progress, locator });
    } catch (error) {
      throw toUserError(error);
    }
  }
}
