import { useCallback, useEffect, useReducer, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { toUserError, type UserFacingError } from '../../lib/errors';
import type { BookSummary } from '../../lib/generated/book';
import { TauriImportIpc } from '../../lib/ipc';
import {
  ImportCoordinator,
  type ImportCoordinatorPort,
} from '../import/ImportCoordinator';
import { ImportButton } from '../import/ImportButton';
import {
  importViewReducer,
  initialImportViewState,
} from '../import/import-view-state';
import { ImportProgress } from '../import/ImportProgress';
import { createDocumentParserRegistry } from '../import/parser-registry';
import { pickTextbook } from '../import/picker';
import { TauriLibraryApi, type LibraryApi } from './api';
import { BookCard } from './BookCard';

export interface LibraryPageProps {
  libraryApi?: LibraryApi;
  importCoordinator?: ImportCoordinatorPort;
}

export function LibraryPage({
  libraryApi,
  importCoordinator,
}: LibraryPageProps = {}) {
  const navigate = useNavigate();
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
  const [importState, dispatch] = useReducer(
    importViewReducer,
    initialImportViewState,
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

  async function runImport(sourcePath: string) {
    dispatch({ type: 'start' });
    try {
      const operation = coordinator.importDocument(
        sourcePath,
        (event) => dispatch({ type: 'progress', event }),
        (book) => dispatch({ type: 'identify', bookId: book.id }),
      );
      const ready = await operation;
      dispatch({ type: 'finish' });
      navigate(`/books/${ready.id}/read`);
    } catch (error) {
      const normalized = toUserError(error);
      dispatch(
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
    dispatch({ type: 'start' });
    try {
      const operation = coordinator.retryDocument(
        bookId,
        sourcePath,
        (event) => dispatch({ type: 'progress', event }),
        (book) => dispatch({ type: 'identify', bookId: book.id }),
      );
      const ready = await operation;
      dispatch({ type: 'finish' });
      navigate(`/books/${ready.id}/read`);
    } catch (error) {
      const normalized = toUserError(error);
      dispatch(
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
      dispatch({ type: 'failed', error: toUserError(error) });
    }
  }

  function cancelImport() {
    if (importState.status !== 'running') return;
    if (importState.bookId) {
      void coordinator.cancel(importState.bookId).catch((error: unknown) => {
        dispatch({ type: 'failed', error: toUserError(error) });
      });
    } else {
      coordinator.cancelPending();
    }
  }

  return (
    <section aria-labelledby="library-title" className="library-page">
      <div className="library-page__header">
        <div>
          <h1 id="library-title">书库</h1>
          <p>教材保存在本机，可离线阅读和检索。</p>
        </div>
        <ImportButton
          disabled={importState.status === 'running'}
          onSelect={runImport}
        />
      </div>

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
            重试加载
          </button>
        </div>
      ) : null}

      {loading ? <p aria-live="polite">正在加载书库…</p> : null}
      {!loading && books.length === 0 ? (
        <p className="empty-state">尚未导入教材。</p>
      ) : null}
      <div className="book-grid" aria-label="教材列表">
        {books.map((book) => (
          <BookCard
            key={book.id}
            book={book}
            onDelete={(bookId) => void deleteFailed(bookId)}
            onOpen={(bookId) => navigate(`/books/${bookId}/read`)}
            onRetry={(bookId) => void retry(bookId)}
          />
        ))}
      </div>
    </section>
  );
}
