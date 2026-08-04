import type { KeyboardEvent } from 'react';

import { useMessage } from '../../app/LanguageProvider';
import type { BookSummary } from '../../lib/generated/book';
import { formatPersistedUtc } from '../../lib/time';

import { isBookOpenable, type LibraryView } from './library-state';

interface LibraryItemProps {
  book: BookSummary;
  focused: boolean;
  selected: boolean;
  view: LibraryView;
  onFocus(bookId: string): void;
  onMoveFocus(bookId: string, offset: -1 | 1): void;
  onOpen(bookId: string): void;
  onRetry(bookId: string): void;
  onDelete(bookId: string): void;
  onSelect(bookId: string): void;
}

export function LibraryItem({
  book,
  focused,
  selected,
  view,
  onFocus,
  onMoveFocus,
  onOpen,
  onRetry,
  onDelete,
  onSelect,
}: LibraryItemProps) {
  const message = useMessage();
  const openable = isBookOpenable(book);
  const id = `library-book-${book.id}`;
  const status = message(`library.importStatus.${book.importStatus}`);
  const indexStatus = message(
    `library.indexStatus.${book.indexAggregate.status}`,
  );
  const lastOpened = book.lastOpenedAt
    ? formatPersistedUtc(book.lastOpenedAt)
    : message('library.neverOpened');

  function handleKeyDown(event: KeyboardEvent<HTMLElement>) {
    if (event.key === ' ') {
      event.preventDefault();
      onSelect(book.id);
      return;
    }
    if (event.key === 'Enter' && openable) {
      event.preventDefault();
      onOpen(book.id);
      return;
    }
    if (event.key === 'ArrowDown' || event.key === 'ArrowRight') {
      event.preventDefault();
      onMoveFocus(book.id, 1);
      return;
    }
    if (event.key === 'ArrowUp' || event.key === 'ArrowLeft') {
      event.preventDefault();
      onMoveFocus(book.id, -1);
    }
  }

  const commonProps = {
    'aria-describedby': `${id}-status`,
    'data-book-id': book.id,
    onClick: () => onSelect(book.id),
    onDoubleClick: () => openable && onOpen(book.id),
    onFocus: () => onFocus(book.id),
    onKeyDown: handleKeyDown,
    tabIndex: focused ? 0 : -1,
  };

  if (view === 'details') {
    return (
      <div {...commonProps} aria-selected={selected} role="row">
        <span role="gridcell">
          {book.title}
          {selected ? <span> ({message('library.selected')})</span> : null}
          <LibraryItemStatus book={book} id={id} />
          {book.importStatus === 'failed' ? (
            <LibraryItemActions
              book={book}
              onDelete={onDelete}
              onRetry={onRetry}
            />
          ) : null}
        </span>
        <span role="gridcell">{book.format.toUpperCase()}</span>
        <span role="gridcell">{status}</span>
        <span role="gridcell">
          {indexStatus}
          {book.indexAggregate.totalPages > 0
            ? ` (${book.indexAggregate.indexedPages}/${book.indexAggregate.totalPages})`
            : ''}
        </span>
        <span role="gridcell">{lastOpened}</span>
      </div>
    );
  }

  return (
    <>
      <button
        {...commonProps}
        aria-label={book.title}
        aria-selected={selected}
        className="library-item"
        role="option"
        type="button"
      >
        <span aria-hidden="true" className="library-item__icon">
          {book.format.toUpperCase()}
        </span>
        <span>{book.title}</span>
        {selected ? <span>{message('library.selected')}</span> : null}
        <span>{book.format.toUpperCase()}</span>
        <span>{status}</span>
        <span>{indexStatus}</span>
        <LibraryItemStatus book={book} id={id} />
      </button>
      {book.importStatus === 'failed' ? (
        <LibraryItemActions book={book} onDelete={onDelete} onRetry={onRetry} />
      ) : null}
    </>
  );
}

function LibraryItemStatus({
  book,
  id,
}: Pick<LibraryItemProps, 'book'> & { id: string }) {
  const message = useMessage();
  if (book.importStatus !== 'failed') {
    return <span id={`${id}-status`} className="library-item__status" />;
  }

  return (
    <span id={`${id}-status`} className="library-item__failure">
      <span>{book.originalFilename}</span>
      <span>{book.importErrorMessage ?? message('library.importFailed')}</span>
    </span>
  );
}

function LibraryItemActions({
  book,
  onDelete,
  onRetry,
}: Pick<LibraryItemProps, 'book' | 'onDelete' | 'onRetry'>) {
  const message = useMessage();
  return (
    <span className="library-item__actions">
      <button type="button" onClick={() => onRetry(book.id)}>
        {message('library.retry')}
      </button>
      <button type="button" onClick={() => onDelete(book.id)}>
        {message('library.deleteFailed')}
      </button>
    </span>
  );
}
