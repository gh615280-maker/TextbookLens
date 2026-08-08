import { useCallback, useEffect, useReducer, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useLanguage, useMessage } from '../../app/LanguageProvider';
import { useModalFocus } from '../../components/useModalFocus';
import { toUserError, type UserFacingError } from '../../lib/errors';
import type { BookSummary } from '../../lib/generated/book';
import type { IndexRunAggregateDto } from '../../lib/generated/indexing';
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
import { TauriIndexingApi } from '../indexing/api';
import { LibraryCommandBar } from './LibraryCommandBar';
import { LibraryDetails } from './LibraryDetails';
import { LibraryDropTarget } from './LibraryDropTarget';
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
  indexRunLookup?: {
    findCurrentRunForBook(bookId: string): Promise<{
      bookId: string;
      runId: string;
      controlStatus?: IndexRunAggregateDto['controlStatus'];
    } | null>;
  };
}

export function LibraryPage({
  libraryApi,
  importCoordinator,
  indexRunLookup,
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
  const [currentRunLookup] = useState(
    () => indexRunLookup ?? new TauriIndexingApi(),
  );
  const [books, setBooks] = useState<BookSummary[]>([]);
  const [activeIndexBookIds, setActiveIndexBookIds] = useState<
    ReadonlySet<string>
  >(() => new Set());
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
  const [removeCandidate, setRemoveCandidate] = useState<BookSummary | null>(
    null,
  );
  const removeDialogRef = useModalFocus<HTMLDivElement>(
    removeCandidate !== null,
    () => setRemoveCandidate(null),
  );

  const readLibrarySnapshot = useCallback(async () => {
    const listedBooks = await api.listBooks();
    const activeRuns = await Promise.all(
      listedBooks
        .filter(
          (book) =>
            book.importStatus === 'ready' &&
            book.format === 'pdf' &&
            book.indexAggregate.status === 'partial',
        )
        .map(async (book) => {
          try {
            const run = await currentRunLookup.findCurrentRunForBook(book.id);
            return run?.bookId === book.id &&
              run.controlStatus !== undefined &&
              isUnfinishedIndexRun(run.controlStatus)
              ? book.id
              : null;
          } catch {
            return null;
          }
        }),
    );
    return {
      books: listedBooks,
      activeIndexBookIds: new Set(
        activeRuns.filter((bookId): bookId is string => bookId !== null),
      ),
    };
  }, [api, currentRunLookup]);

  const refresh = useCallback(async () => {
    try {
      const snapshot = await readLibrarySnapshot();
      setBooks(snapshot.books);
      setActiveIndexBookIds(snapshot.activeIndexBookIds);
      setLoadError(null);
    } catch (error) {
      setLoadError(toUserError(error));
    } finally {
      setLoading(false);
    }
  }, [readLibrarySnapshot]);

  useEffect(() => {
    let active = true;
    void readLibrarySnapshot()
      .then((snapshot) => {
        if (!active) return;
        setBooks(snapshot.books);
        setActiveIndexBookIds(snapshot.activeIndexBookIds);
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
  }, [readLibrarySnapshot]);

  useEffect(() => {
    if (activeIndexBookIds.size === 0) return;
    let active = true;
    let polling = false;
    const timer = window.setInterval(() => {
      if (polling) return;
      polling = true;
      void readLibrarySnapshot()
        .then((snapshot) =>
          snapshot.activeIndexBookIds.size === 0
            ? readLibrarySnapshot().catch(() => snapshot)
            : snapshot,
        )
        .then((snapshot) => {
          if (!active) return;
          setBooks(snapshot.books);
          setActiveIndexBookIds(snapshot.activeIndexBookIds);
          setLoadError(null);
        })
        .catch(() => {
          // Keep the last safe library snapshot on a transient poll failure.
        })
        .finally(() => {
          polling = false;
        });
    }, 3_000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [activeIndexBookIds.size, readLibrarySnapshot]);

  const visibleBooks = getVisibleBooks(
    books,
    libraryState,
    uiLanguage,
    activeIndexBookIds,
  );

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
        (book) => {
          // This is safe Rust-returned metadata, not the selected source path.
          // Showing it immediately keeps durable imports visible across navigation.
          setBooks((current) => upsertBook(current, book));
          dispatchImport({ type: 'identify', bookId: book.id });
          dispatchLibrary({ type: 'select', bookId: book.id });
        },
      );
      const ready = await operation;
      setBooks((current) => upsertBook(current, ready));
      dispatchImport({ type: 'finish' });
      dispatchLibrary({ type: 'select', bookId: ready.id });
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

  async function runDroppedImports(sourcePaths: readonly string[]) {
    // Keep the OS-provided paths on this stack only. Each coordinator call hands
    // the path straight to Rust, which verifies the file and owns the app copy.
    for (const sourcePath of sourcePaths) await runImport(sourcePath);
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

  async function showIndexStatus(book: BookSummary) {
    if (book.indexAggregate.status === 'not_required') return;
    try {
      const aggregate = await currentRunLookup.findCurrentRunForBook(book.id);
      if (!aggregate || aggregate.bookId !== book.id) {
        throw new Error('The resolved index run does not belong to this book.');
      }
      navigate(
        `/books/${encodeURIComponent(book.id)}/index-quality/${encodeURIComponent(aggregate.runId)}`,
      );
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
    onDeleteFailed: (bookId: string) => void deleteFailed(bookId),
    onFocus: (bookId: string) => dispatchLibrary({ type: 'focus', bookId }),
    onIndexStatus: (book: BookSummary) => void showIndexStatus(book),
    onStartIndex: (book: BookSummary) =>
      navigate(`/books/${encodeURIComponent(book.id)}/index-start`),
    onMoveFocus: moveFocus,
    onOpen: (bookId: string) => navigate(`/books/${bookId}/read`),
    onRequestRemove: (book: BookSummary) => {
      if (book.importStatus === 'failed') setRemoveCandidate(book);
    },
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
      {removeCandidate ? (
        <div
          ref={removeDialogRef}
          aria-labelledby="library-remove-title"
          aria-modal="true"
          role="dialog"
        >
          <h2 id="library-remove-title">{message('library.remove.title')}</h2>
          <p>{message('library.remove.originalUnaffected')}</p>
          <p>{removeCandidate.title}</p>
          <button type="button" onClick={() => setRemoveCandidate(null)}>
            {message('library.remove.cancel')}
          </button>
          <button
            type="button"
            onClick={() => {
              const bookId = removeCandidate.id;
              setRemoveCandidate(null);
              void deleteFailed(bookId);
            }}
          >
            {message('library.remove.confirm')}
          </button>
        </div>
      ) : null}

      <LibraryDropTarget onDrop={runDroppedImports}>
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
      </LibraryDropTarget>
    </section>
  );
}

function upsertBook(
  current: readonly BookSummary[],
  next: BookSummary,
): BookSummary[] {
  const index = current.findIndex((book) => book.id === next.id);
  if (index === -1) return [next, ...current];
  return current.map((book) => (book.id === next.id ? next : book));
}

function isUnfinishedIndexRun(
  status: IndexRunAggregateDto['controlStatus'],
): boolean {
  return status === 'running' || status === 'paused' || status === 'cancelling';
}
