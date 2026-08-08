import { invoke } from '@tauri-apps/api/core';

import { toUserError } from '../../lib/errors';
import type { BookSummary } from '../../lib/generated/book';
import type { DocumentLocator } from '../../lib/generated/document';

export type ReaderTheme = 'light' | 'dark' | 'system';

export interface ReaderSettings {
  fontScale: number;
  lineHeight: number;
  readerWidth: number;
  pdfZoom: number;
  theme: ReaderTheme;
}

export interface ReaderSection {
  id: string;
  parentId: string | null;
  ordinal: number;
  title: string;
  locator: DocumentLocator;
}

export interface ReaderSearchHit {
  snippet: string;
  locator: DocumentLocator;
  sectionTitle: string | null;
  source?: 'book' | 'question';
}

export type ReaderSearchScope = 'book' | 'question' | 'all';

export interface AnnotationMarkerDto {
  id: string;
  kind: 'ai_conversation' | 'note';
  conversationId: string | null;
  anchor: import('../../lib/generated/document').ContentAnchor | null;
  relocationStatus: 'primary' | 'fallback' | 'unresolved';
  accessibilityLabel: string;
  sequence?: number | null;
  summaryText?: string;
  revision?: number;
}

export interface ReaderAnnotationMarkerDto {
  id: string;
  kind: 'ai_conversation' | 'note';
  conversationId: string | null;
  anchor: import('../../lib/generated/document').ContentAnchor | null;
  relocationStatus: 'primary' | 'fallback' | 'unresolved';
  accessibilityLabel: string;
  sequence?: number | null;
  summaryText?: string;
  revision?: number;
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
  saveReadingProgress(
    bookId: string,
    progress: number,
    locator: DocumentLocator,
  ): Promise<void>;
  listReaderSections(bookId: string): Promise<ReaderSection[]>;
  ensurePdfPageSections?(
    bookId: string,
    pageCount: number,
  ): Promise<ReaderSection[]>;
  searchBook(
    bookId: string,
    query: string,
    limit: number,
    scope?: ReaderSearchScope,
  ): Promise<ReaderSearchHit[]>;
  listAnnotationMarkers(bookId: string): Promise<ReaderAnnotationMarkerDto[]>;
  updateAiAnnotationSummary?(
    bookId: string,
    annotationId: string,
    expectedRevision: number,
    summaryText: string,
  ): Promise<void>;
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
      throw new TypeError(
        'read_book_source returned an invalid binary payload',
      );
    } catch (error) {
      throw toUserError(error);
    }
  }

  async readDerivedText(
    bookId: string,
    name: 'document.html',
  ): Promise<string> {
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

  async updateReaderSettings(
    settings: ReaderSettings,
  ): Promise<ReaderSettings> {
    try {
      return await invoke<ReaderSettings>('update_reader_settings', {
        settings,
      });
    } catch (error) {
      throw toUserError(error);
    }
  }

  async saveReadingProgress(
    bookId: string,
    progress: number,
    locator: DocumentLocator,
  ): Promise<void> {
    try {
      await invoke('save_reading_progress', { bookId, progress, locator });
    } catch (error) {
      throw toUserError(error);
    }
  }

  async listReaderSections(bookId: string): Promise<ReaderSection[]> {
    try {
      return await invoke<ReaderSection[]>('list_reader_sections', { bookId });
    } catch (error) {
      throw toUserError(error);
    }
  }

  async ensurePdfPageSections(
    bookId: string,
    pageCount: number,
  ): Promise<ReaderSection[]> {
    try {
      return await invoke<ReaderSection[]>('ensure_pdf_page_sections', {
        bookId,
        pageCount,
      });
    } catch (error) {
      throw toUserError(error);
    }
  }

  async searchBook(
    bookId: string,
    query: string,
    limit: number,
    scope: ReaderSearchScope = 'all',
  ): Promise<ReaderSearchHit[]> {
    try {
      const cappedLimit = Math.min(50, limit);
      const [bookHits, markers] = await Promise.all([
        scope === 'question'
          ? Promise.resolve([])
          : invoke<ReaderSearchHit[]>('search_book', {
              bookId,
              query,
              limit: cappedLimit,
            }),
        scope === 'book'
          ? Promise.resolve([])
          : this.listAnnotationMarkers(bookId),
      ]);
      const normalizedQuery = query.trim().toLocaleLowerCase();
      const questionHits = markers
        .filter(
          (marker) =>
            marker.kind === 'ai_conversation' &&
            marker.summaryText?.toLocaleLowerCase().includes(normalizedQuery),
        )
        .flatMap((marker): ReaderSearchHit[] => {
          const locator = marker.anchor
            ? locatorFromAnchor(marker.anchor)
            : null;
          return locator
            ? [
                {
                  snippet: marker.summaryText ?? '',
                  locator,
                  sectionTitle: null,
                  source: 'question',
                },
              ]
            : [];
        });
      return [
        ...questionHits,
        ...bookHits.map((hit) => ({ ...hit, source: 'book' as const })),
      ].slice(0, cappedLimit);
    } catch (error) {
      throw toUserError(error);
    }
  }

  async listAnnotationMarkers(
    bookId: string,
  ): Promise<ReaderAnnotationMarkerDto[]> {
    try {
      const markers = await invoke<AnnotationMarkerDto[]>(
        'list_annotation_markers',
        {
          bookId,
        },
      );
      return markers.map((marker) => ({
        id: marker.id,
        kind: marker.kind,
        conversationId: marker.conversationId,
        anchor: marker.anchor,
        relocationStatus: marker.relocationStatus,
        accessibilityLabel: marker.accessibilityLabel,
        sequence: marker.sequence,
        summaryText: marker.summaryText,
        revision: marker.revision,
      }));
    } catch (error) {
      throw toUserError(error);
    }
  }

  async updateAiAnnotationSummary(
    bookId: string,
    annotationId: string,
    expectedRevision: number,
    summaryText: string,
  ): Promise<void> {
    try {
      await invoke('update_ai_annotation_summary', {
        bookId,
        annotationId,
        expectedRevision,
        summaryText,
      });
    } catch (error) {
      throw toUserError(error);
    }
  }
}

function locatorFromAnchor(
  anchor: import('../../lib/generated/document').ContentAnchor,
): DocumentLocator | null {
  if (anchor.kind === 'text') return anchor.selection.locator;
  const locator = anchor.region.locator;
  if (locator.format === 'pdf') {
    return {
      format: 'pdf',
      startPage: locator.page,
      endPage: locator.page,
      rectsByPage: null,
    };
  }
  if (locator.format === 'epub') return locator;
  return {
    format: 'docx',
    startBlockId: locator.blockId,
    startOffset: 0,
    endBlockId: locator.blockId,
    endOffset: 0,
  };
}
