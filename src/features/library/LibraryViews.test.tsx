import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useEffect, useState, type ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { LanguageContext } from '../../app/LanguageProvider';
import { formatMessage } from '../../lib/i18n';
import type { BookSummary } from '../../lib/generated/book';
import { LibraryDetails } from './LibraryDetails';
import { LibraryGrid } from './LibraryGrid';

function book(overrides: Partial<BookSummary> = {}): BookSummary {
  return {
    id: 'book-a',
    title: 'Alpha',
    originalFilename: 'alpha.pdf',
    author: 'Author',
    language: 'en',
    format: 'pdf',
    importStatus: 'ready',
    importErrorCode: null,
    importErrorMessage: null,
    importErrorStage: null,
    readingProgress: 0,
    fullTextQaReady: false,
    indexAggregate: {
      status: 'partial',
      totalPages: 10,
      indexedPages: 8,
      reviewPages: 1,
      failedPages: 1,
    },
    createdAt: '2026-08-01T00:00:00Z',
    updatedAt: '2026-08-01T00:00:00Z',
    lastOpenedAt: null,
    ...overrides,
  };
}

function WithLanguage({ children }: { children: ReactNode }) {
  return (
    <LanguageContext.Provider
      value={{
        uiLanguage: 'en',
        isLoading: false,
        statusMessage: null,
        switchLanguage: vi.fn(),
        message: (key, values) => formatMessage('en', key, values),
      }}
    >
      {children}
    </LanguageContext.Provider>
  );
}

function KeyboardGrid({ onOpen }: { onOpen: (bookId: string) => void }) {
  const books = [book(), book({ id: 'book-b', title: 'Beta' })];
  const [focusedBookId, setFocusedBookId] = useState<string | null>(null);
  const [selectedBookId, setSelectedBookId] = useState<string | null>(null);
  useEffect(() => {
    if (!focusedBookId) return;
    const item = Array.from(
      document.querySelectorAll<HTMLElement>('[data-book-id]'),
    ).find((element) => element.dataset.bookId === focusedBookId);
    item?.focus();
  }, [focusedBookId]);
  return (
    <LibraryGrid
      books={books}
      focusedBookId={focusedBookId}
      selectedBookId={selectedBookId}
      label="Library items"
      onFocus={setFocusedBookId}
      onDeleteFailed={vi.fn()}
      onIndexStatus={vi.fn()}
      onStartIndex={vi.fn()}
      onMoveFocus={(bookId, offset) => {
        const index = books.findIndex((candidate) => candidate.id === bookId);
        const next = books[index + offset];
        if (next) setFocusedBookId(next.id);
      }}
      onOpen={onOpen}
      onRequestRemove={vi.fn()}
      onRetry={vi.fn()}
      onSelect={setSelectedBookId}
    />
  );
}

afterEach(cleanup);

describe('library views', () => {
  it('keeps keyboard focus separate from selection and opens ready partial books', async () => {
    const user = userEvent.setup();
    const onOpen = vi.fn();
    render(
      <WithLanguage>
        <KeyboardGrid onOpen={onOpen} />
      </WithLanguage>,
    );

    const alpha = screen.getByRole('button', { name: 'Alpha' });
    const beta = screen.getByRole('button', { name: 'Beta' });
    alpha.focus();
    await user.keyboard('{ArrowRight}');
    expect(beta).toHaveFocus();
    expect(alpha).toHaveAttribute('aria-pressed', 'false');

    await user.keyboard(' ');
    expect(beta).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByText('Selected')).toBeVisible();
    await user.keyboard('{Enter}');
    expect(onOpen).toHaveBeenCalledWith('book-b');
  });

  it('uses the same safe metadata model in details columns and keeps failed imports non-openable', async () => {
    const user = userEvent.setup();
    const onOpen = vi.fn();
    render(
      <WithLanguage>
        <LibraryDetails
          books={[
            book({
              importStatus: 'failed',
              indexAggregate: { ...book().indexAggregate, status: 'failed' },
            }),
          ]}
          focusedBookId={null}
          selectedBookId={null}
          onFocus={vi.fn()}
          onDeleteFailed={vi.fn()}
          onIndexStatus={vi.fn()}
          onStartIndex={vi.fn()}
          onMoveFocus={vi.fn()}
          onOpen={onOpen}
          onRequestRemove={vi.fn()}
          onRetry={vi.fn()}
          onSelect={vi.fn()}
        />
      </WithLanguage>,
    );

    expect(screen.getByRole('grid')).toBeVisible();
    expect(
      screen.getAllByRole('columnheader').map((cell) => cell.textContent),
    ).toEqual([
      'Title',
      'Format',
      'Import progress',
      'Index status',
      'Last opened',
    ]);
    const row = screen.getAllByRole('row')[1];
    row.focus();
    await user.keyboard('{Enter}');
    expect(onOpen).not.toHaveBeenCalled();
  });
});
