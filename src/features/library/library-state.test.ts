import { describe, expect, it } from 'vitest';

import type { BookSummary } from '../../lib/generated/book';
import {
  getVisibleBooks,
  initialLibraryState,
  isBookOpenable,
  libraryStateReducer,
} from './library-state';

function book(overrides: Partial<BookSummary> = {}): BookSummary {
  return {
    id: 'b',
    title: 'Algebra 10',
    originalFilename: 'algebra.pdf',
    author: null,
    language: 'en',
    format: 'pdf',
    importStatus: 'ready',
    importErrorCode: null,
    importErrorMessage: null,
    importErrorStage: null,
    readingProgress: 0,
    fullTextQaReady: false,
    indexAggregate: {
      status: 'not_required',
      totalPages: 0,
      indexedPages: 0,
      reviewPages: 0,
      failedPages: 0,
    },
    createdAt: '2026-08-01T00:00:00Z',
    updatedAt: '2026-08-01T00:00:00Z',
    lastOpenedAt: null,
    ...overrides,
  };
}

describe('library state', () => {
  it('keeps only view state and records selection separately from focus', () => {
    const selected = libraryStateReducer(initialLibraryState, {
      type: 'select',
      bookId: 'book-1',
    });
    const focused = libraryStateReducer(selected, {
      type: 'focus',
      bookId: 'book-2',
    });

    expect(selected).toMatchObject({
      selectedBookId: 'book-1',
      focusedBookId: 'book-1',
    });
    expect(focused).toMatchObject({
      selectedBookId: 'book-1',
      focusedBookId: 'book-2',
    });
    expect(Object.keys(focused)).toEqual([
      'filter',
      'query',
      'sort',
      'view',
      'selectedBookId',
      'focusedBookId',
    ]);
  });

  it('searches safe title, original filename, and author metadata with locale folding', () => {
    const books = [
      book({ id: 'title', title: 'İSTANBUL geography' }),
      book({ id: 'filename', originalFilename: 'résumé.epub' }),
      book({ id: 'author', author: 'Ada Lovelace' }),
    ];

    expect(
      getVisibleBooks(
        books,
        { ...initialLibraryState, query: 'istanbul' },
        'tr',
      ),
    ).toHaveLength(1);
    expect(
      getVisibleBooks(books, { ...initialLibraryState, query: 'RÉSUMÉ' }, 'fr'),
    ).toHaveLength(1);
    expect(
      getVisibleBooks(books, { ...initialLibraryState, query: 'ada' }, 'en'),
    ).toHaveLength(1);
  });

  it('filters processing rows and sorts deterministically with the book id tie-breaker', () => {
    const books = [
      book({ id: 'z', title: 'Same', importStatus: 'copying' }),
      book({ id: 'a', title: 'Same', importStatus: 'parsing' }),
      book({ id: 'ready', title: 'Same' }),
    ];

    expect(
      getVisibleBooks(
        books,
        { ...initialLibraryState, filter: 'processing' },
        'en',
      ).map((candidate) => candidate.id),
    ).toEqual(['a', 'z']);
  });

  it('includes locally ready books while their AI index run is unfinished', () => {
    const indexing = book({ id: 'ai-running', title: 'AI running' });
    const completed = book({ id: 'completed', title: 'Completed' });

    expect(
      getVisibleBooks(
        [completed, indexing],
        { ...initialLibraryState, filter: 'processing' },
        'en',
        new Set([indexing.id]),
      ).map((candidate) => candidate.id),
    ).toEqual(['ai-running']);
  });

  it('keeps locally ready partial and review books openable', () => {
    expect(
      isBookOpenable(
        book({
          indexAggregate: { ...book().indexAggregate, status: 'partial' },
        }),
      ),
    ).toBe(true);
    expect(
      isBookOpenable(
        book({
          indexAggregate: { ...book().indexAggregate, status: 'needs_review' },
        }),
      ),
    ).toBe(true);
    expect(isBookOpenable(book({ importStatus: 'parsing' }))).toBe(false);
  });
});
