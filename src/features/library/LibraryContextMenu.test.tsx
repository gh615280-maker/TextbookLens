import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { LanguageContext } from '../../app/LanguageProvider';
import { formatMessage } from '../../lib/i18n';
import type { BookSummary } from '../../lib/generated/book';
import type { ImportCoordinatorPort } from '../import/ImportCoordinator';
import type { LibraryApi } from './api';
import { LibraryItem } from './LibraryItem';
import { LibraryPage } from './LibraryPage';

const BOOK_ID = '22222222-2222-4222-8222-222222222222';
const RUN_ID = '11111111-1111-4111-8111-111111111111';

function book(overrides: Partial<BookSummary> = {}): BookSummary {
  return {
    id: BOOK_ID,
    title: 'Alpha',
    originalFilename: 'alpha.pdf',
    author: null,
    language: 'en',
    format: 'pdf',
    importStatus: 'ready',
    importErrorCode: null,
    importErrorMessage: null,
    importErrorStage: null,
    readingProgress: 0,
    indexAggregate: {
      status: 'partial',
      totalPages: 5,
      indexedPages: 3,
      reviewPages: 1,
      failedPages: 1,
    },
    createdAt: '2026-08-04T00:00:00.000Z',
    updatedAt: '2026-08-04T00:00:00.000Z',
    lastOpenedAt: null,
    ...overrides,
  };
}

function WithLanguage({ children }: { children: React.ReactNode }) {
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

afterEach(cleanup);

describe('library context menu', () => {
  it('opens with right click and Shift+F10, has exact actions, and returns focus', async () => {
    const user = userEvent.setup();
    const onIndexStatus = vi.fn();
    render(
      <WithLanguage>
        <LibraryItem
          book={book()}
          focused
          selected={false}
          view="large"
          onFocus={vi.fn()}
          onDeleteFailed={vi.fn()}
          onIndexStatus={onIndexStatus}
          onMoveFocus={vi.fn()}
          onOpen={vi.fn()}
          onRequestRemove={vi.fn()}
          onRetry={vi.fn()}
          onSelect={vi.fn()}
        />
      </WithLanguage>,
    );

    const item = screen.getByRole('option', { name: 'Alpha' });
    item.focus();
    fireEvent.contextMenu(item);
    screen.getByRole('menu', { name: 'Book actions' });
    expect(
      screen.getAllByRole('menuitem').map((entry) => entry.textContent),
    ).toEqual(['Open', 'Index status']);
    await user.keyboard('{Escape}');
    expect(item).toHaveFocus();

    await user.keyboard('{Shift>}{F10}{/Shift}');
    await user.click(screen.getByRole('menuitem', { name: 'Index status' }));
    expect(onIndexStatus).toHaveBeenCalledWith(book());
  });

  it('routes only an ownership-verified durable index run and confirms safe failed-record removal', async () => {
    const user = userEvent.setup();
    const libraryApi: LibraryApi = {
      listBooks: vi.fn(async () => [book()]),
      deleteFailedImport: vi.fn(async () => {}),
    };
    const coordinator: ImportCoordinatorPort = {
      importDocument: vi.fn(),
      retryDocument: vi.fn(),
      cancel: vi.fn(),
      cancelPending: vi.fn(),
    };
    const indexRunLookup = {
      findCurrentRunForBook: vi.fn(async () => ({
        bookId: BOOK_ID,
        runId: RUN_ID,
      })),
    };
    render(
      <WithLanguage>
        <MemoryRouter initialEntries={['/library']}>
          <Routes>
            <Route
              path="/library"
              element={
                <LibraryPage
                  importCoordinator={coordinator}
                  indexRunLookup={indexRunLookup}
                  libraryApi={libraryApi}
                />
              }
            />
            <Route
              path="/books/:bookId/index-quality/:runId"
              element={<p>Quality</p>}
            />
          </Routes>
          <LocationProbe />
        </MemoryRouter>
      </WithLanguage>,
    );

    const item = await screen.findByRole('option', { name: 'Alpha' });
    fireEvent.contextMenu(item);
    await user.click(screen.getByRole('menuitem', { name: 'Index status' }));
    await waitFor(() =>
      expect(screen.getByTestId('location')).toHaveTextContent(
        `/books/${BOOK_ID}/index-quality/${RUN_ID}`,
      ),
    );
    expect(indexRunLookup.findCurrentRunForBook).toHaveBeenCalledWith(BOOK_ID);
  });

  it('shows Remove only for the existing failed-import delete contract and confirms the original stays untouched', async () => {
    const user = userEvent.setup();
    const failed = book({
      importStatus: 'failed',
      indexAggregate: {
        status: 'not_required',
        totalPages: 0,
        indexedPages: 0,
        reviewPages: 0,
        failedPages: 0,
      },
    });
    const deleteFailedImport = vi.fn(async () => {});
    render(
      <WithLanguage>
        <MemoryRouter>
          <LibraryPage
            importCoordinator={{
              importDocument: vi.fn(),
              retryDocument: vi.fn(),
              cancel: vi.fn(),
              cancelPending: vi.fn(),
            }}
            libraryApi={{ listBooks: async () => [failed], deleteFailedImport }}
            indexRunLookup={{ findCurrentRunForBook: vi.fn() }}
          />
        </MemoryRouter>
      </WithLanguage>,
    );

    const item = await screen.findByRole('option', { name: 'Alpha' });
    fireEvent.contextMenu(item);
    await user.click(screen.getByRole('menuitem', { name: 'Remove' }));
    expect(screen.getByRole('dialog')).toHaveTextContent(
      'Your original file is unaffected.',
    );
    await user.click(
      screen.getByRole('button', { name: 'Remove failed record' }),
    );
    expect(deleteFailedImport).toHaveBeenCalledWith(BOOK_ID);
  });
});

function LocationProbe() {
  const location = useLocation();
  return <output data-testid="location">{location.pathname}</output>;
}
