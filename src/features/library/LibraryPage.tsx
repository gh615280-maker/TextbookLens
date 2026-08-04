import { useCallback, useEffect, useReducer, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useLanguage, useMessage } from '../../app/LanguageProvider';
import { toUserError, type UserFacingError } from '../../lib/errors';
import type { BookSummary } from '../../lib/generated/book';
import { TauriImportIpc } from '../../lib/ipc';
import {
  ImportCoordinator,
  type ImportCoordinatorPort,
} from '../import/ImportCoordinator';
import {
  importViewReducer,
  initialImportViewState,
} from '../import/import-view-state';
import { ImportProgress } from '../import/ImportProgress';
import { createDocumentParserRegistry } from '../import/parser-registry';
import { pickTextbook } from '../import/picker';
import { TauriLibraryApi, type LibraryApi } from './api';
import { LibraryCommandBar } from './LibraryCommandBar';
import { LibraryDetails } from './LibraryDetails';
import { LibraryGrid } from './LibraryGrid';
import {
  getVisibleBooks,
  initialLibraryState,
  libraryStateReducer,
} from './library-state';
import { LibrarySidebar } from './LibrarySidebar';

export interface LibraryPageProps {
  libraryApi?: LibraryApi;
  importCoordinator?: ImportCoordinatorPort;
}

export function LibraryPage({
  libraryApi,
  importCoordinator,
}: LibraryPageProps = {}) {
  const navigate = useNavigate();
  const message = useMessage();
  const { uiLanguage } = useLanguage();
  const [api] = useState<LibraryApi>(() => libraryApi ?? new TauriLibraryApi());
  const [coordinator] = useState<ImportCoordinatorPort>(
    () =>
      importCoordinator ??
      new ImportCoordinator(
        new TauriImportIpc(),
        createDocumentParserRegistry(),
      ),
  );
  const [books, setBooks] = useState<BookSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<UserFacingError | null>(null);
  const [importState, dispatchImport] = useReducer(
    importViewReducer,
    initialImportViewState,
  );
  const [libraryState, dispatchLibrary] = useReducer(
    libraryStateReducer,
    initialLibraryState,
  );

  const refresh = useCallback(async () => {
    try {
      setBooks(await api.listBooks());
      setLoadError(null);
    } catch (error) {
      setLoadError(toUserError(error));
    } finally {
      setLoading(false);
    }
  }, [api]);

  useEffect(() => {
    let active = true;
    void api
      .listBooks()
      .then((listedBooks) => {
        if (!active) return;
        setBooks(listedBooks);
        setLoadError(null);
      })
      .catch((error: unknown) => {
        if (active) setLoadError(toUserError(error));
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [api]);

  const visibleBooks = getVisibleBooks(books, libraryState, uiLanguage);

  useEffect(() => {
    if (!libraryState.focusedBookId) return;
    const next = Array.from(
      document.querySelectorAll<HTMLElement>('[data-book-id]'),
    ).find((element) => element.dataset.bookId === libraryState.focusedBookId);
    next?.focus();
  }, [libraryState.focusedBookId, libraryState.view, visibleBooks]);

  async function runImport(sourcePath: string) {
    dispatchImport({ type: 'start' });
    try {
      const operation = coordinator.importDocument(
        sourcePath,
        (event) => dispatchImport({ type: 'progress', event }),
        (book) => dispatchImport({ type: 'identify', bookId: book.id }),
      );
      const ready = await operation;
      dispatchImport({ type: 'finish' });
      navigate(`/books/${ready.id}/read`);
    } catch (error) {
      const normalized = toUserError(error);
      dispatchImport(
        normalized.code === 'IMPORT_CANCELLED'
          ? { type: 'finish' }
          : { type: 'failed', error: normalized },
      );
      await refresh();
    }
  }

  async function retry(bookId: string) {
    const sourcePath = await pickTextbook();
    if (!sourcePath) return;
    dispatchImport({ type: 'start' });
    try {
      const operation = coordinator.retryDocument(
        bookId,
        sourcePath,
        (event) => dispatchImport({ type: 'progress', event }),
        (book) => dispatchImport({ type: 'identify', bookId: book.id }),
      );
      const ready = await operation;
      dispatchImport({ type: 'finish' });
      navigate(`/books/${ready.id}/read`);
    } catch (error) {
      const normalized = toUserError(error);
      dispatchImport(
        normalized.code === 'IMPORT_CANCELLED'
          ? { type: 'finish' }
          : { type: 'failed', error: normalized },
      );
      await refresh();
    }
  }

  async function deleteFailed(bookId: string) {
    try {
      await api.deleteFailedImport(bookId);
      await refresh();
    } catch (error) {
      dispatchImport({ type: 'failed', error: toUserError(error) });
    }
  }

  function cancelImport() {
    if (importState.status !== 'running') return;
    if (importState.bookId) {
      void coordinator.cancel(importState.bookId).catch((error: unknown) => {
        dispatchImport({ type: 'failed', error: toUserError(error) });
      });
    } else {
      coordinator.cancelPending();
    }
  }

  function moveFocus(bookId: string, offset: -1 | 1) {
    const index = visibleBooks.findIndex((book) => book.id === bookId);
    if (index < 0) return;
    const next = visibleBooks[index + offset];
    if (next) dispatchLibrary({ type: 'focus', bookId: next.id });
  }

  const itemActions = {
    onDelete: (bookId: string) => void deleteFailed(bookId),
    onFocus: (bookId: string) => dispatchLibrary({ type: 'focus', bookId }),
    onMoveFocus: moveFocus,
    onOpen: (bookId: string) => navigate(`/books/${bookId}/read`),
    onRetry: (bookId: string) => void retry(bookId),
    onSelect: (bookId: string) => dispatchLibrary({ type: 'select', bookId }),
  };

  return (
    <section aria-labelledby="library-title" className="library-page">
      <header className="library-page__header">
        <h1 id="library-title">{message('page.library.title')}</h1>
      </header>
      <LibraryCommandBar
        filter={libraryState.filter}
        importDisabled={importState.status === 'running'}
        query={libraryState.query}
        sort={libraryState.sort}
        view={libraryState.view}
        onImport={runImport}
        onQueryChange={(query) => dispatchLibrary({ type: 'setQuery', query })}
        onSortChange={(sort) => dispatchLibrary({ type: 'setSort', sort })}
        onViewChange={(view) => dispatchLibrary({ type: 'setView', view })}
      />

      {importState.status === 'running' ? (
        <ImportProgress event={importState.event} onCancel={cancelImport} />
      ) : null}
      {importState.status === 'failed' ? (
        <div className="inline-error" role="alert">
          <p>{importState.error.message}</p>
          <p>{importState.error.nextStep}</p>
        </div>
      ) : null}
      {loadError ? (
        <div className="inline-error" role="alert">
          <p>{loadError.message}</p>
          <button type="button" onClick={() => void refresh()}>
            {message('library.retryLoading')}
          </button>
        </div>
      ) : null}

      <div
        className="library-explorer"
        style={{ display: 'flex', flexWrap: 'wrap', gap: 'var(--space-4)' }}
      >
        <LibrarySidebar
          filter={libraryState.filter}
          onFilterChange={(filter) =>
            dispatchLibrary({ type: 'setFilter', filter })
          }
        />
        <div
          className="library-explorer__content"
          style={{ flex: '999 1 32rem', minWidth: 0 }}
        >
          {loading ? (
            <p aria-live="polite">{message('library.loading')}</p>
          ) : null}
          {!loading && visibleBooks.length === 0 ? (
            <p className="empty-state">{message('library.empty')}</p>
          ) : null}
          {!loading &&
          visibleBooks.length > 0 &&
          libraryState.view === 'large' ? (
            <LibraryGrid
              books={visibleBooks}
              focusedBookId={libraryState.focusedBookId}
              selectedBookId={libraryState.selectedBookId}
              label={message('library.itemsLabel')}
              {...itemActions}
            />
          ) : null}
          {!loading &&
          visibleBooks.length > 0 &&
          libraryState.view === 'details' ? (
            <LibraryDetails
              books={visibleBooks}
              focusedBookId={libraryState.focusedBookId}
              selectedBookId={libraryState.selectedBookId}
              {...itemActions}
            />
          ) : null}
        </div>
      </div>
    </section>
  );
}
