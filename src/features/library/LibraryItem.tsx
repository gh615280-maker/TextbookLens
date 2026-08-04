import { useRef, useState, type KeyboardEvent } from 'react';

import { useMessage } from '../../app/LanguageProvider';
import type { BookSummary } from '../../lib/generated/book';
import { formatPersistedUtc } from '../../lib/time';

import { isBookOpenable, type LibraryView } from './library-state';
import { BookIndexStatus } from './BookIndexStatus';
import { LibraryContextMenu } from './LibraryContextMenu';

interface LibraryItemProps {
  book: BookSummary;
  focused: boolean;
  selected: boolean;
  view: LibraryView;
  onFocus(bookId: string): void;
  onDeleteFailed(bookId: string): void;
  onMoveFocus(bookId: string, offset: -1 | 1): void;
  onOpen(bookId: string): void;
  onRetry(bookId: string): void;
  onIndexStatus(book: BookSummary): void;
  onRequestRemove(book: BookSummary): void;
  onSelect(bookId: string): void;
}

export function LibraryItem({
  book,
  focused,
  selected,
  view,
  onFocus,
  onDeleteFailed,
  onMoveFocus,
  onOpen,
  onRetry,
  onIndexStatus,
  onRequestRemove,
  onSelect,
}: LibraryItemProps) {
  const message = useMessage();
  const triggerRef = useRef<HTMLElement>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const openable = isBookOpenable(book);
  const id = `library-book-${book.id}`;
  const status = message(`library.importStatus.${book.importStatus}`);
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
    if (
      event.key === 'ContextMenu' ||
      (event.shiftKey && event.key === 'F10')
    ) {
      event.preventDefault();
      setMenuOpen(true);
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
    onContextMenu: (event: { preventDefault(): void }) => {
      event.preventDefault();
      setMenuOpen(true);
    },
    tabIndex: focused ? 0 : -1,
  };

  if (view === 'details') {
    return (
      <>
        <div
          {...commonProps}
          aria-selected={selected}
          ref={(element) => {
            triggerRef.current = element;
          }}
          role="row"
        >
          <span role="gridcell">
            {book.title}
            {selected ? <span> ({message('library.selected')})</span> : null}
            <LibraryItemStatus book={book} id={id} />
            {book.importStatus === 'failed' ? (
              <LibraryItemActions
                book={book}
                onDeleteFailed={onDeleteFailed}
                onRetry={onRetry}
              />
            ) : null}
          </span>
          <span role="gridcell">{book.format.toUpperCase()}</span>
          <span role="gridcell">{status}</span>
          <span role="gridcell">
            <BookIndexStatus book={book} onOpenStatus={onIndexStatus} />
          </span>
          <span role="gridcell">{lastOpened}</span>
        </div>
        {menuOpen ? (
          <LibraryContextMenu
            canOpen={openable}
            canRemove={book.importStatus === 'failed'}
            canShowIndexStatus={book.indexAggregate.status !== 'not_required'}
            onClose={() => {
              setMenuOpen(false);
              triggerRef.current?.focus();
            }}
            onOpen={() => onOpen(book.id)}
            onRemove={() => onRequestRemove(book)}
            onShowIndexStatus={() => onIndexStatus(book)}
          />
        ) : null}
      </>
    );
  }

  return (
    <>
      <div
        {...commonProps}
        aria-label={book.title}
        aria-selected={selected}
        className="library-item"
        ref={(element) => {
          triggerRef.current = element;
        }}
        role="option"
      >
        <span aria-hidden="true" className="library-item__icon">
          {book.format.toUpperCase()}
        </span>
        <span>{book.title}</span>
        {selected ? <span>{message('library.selected')}</span> : null}
        <span>{book.format.toUpperCase()}</span>
        <span>{status}</span>
        <BookIndexStatus book={book} onOpenStatus={onIndexStatus} />
        <LibraryItemStatus book={book} id={id} />
      </div>
      {book.importStatus === 'failed' ? (
        <LibraryItemActions
          book={book}
          onDeleteFailed={onDeleteFailed}
          onRetry={onRetry}
        />
      ) : null}
      {menuOpen ? (
        <LibraryContextMenu
          canOpen={openable}
          canRemove={book.importStatus === 'failed'}
          canShowIndexStatus={book.indexAggregate.status !== 'not_required'}
          onClose={() => {
            setMenuOpen(false);
            triggerRef.current?.focus();
          }}
          onOpen={() => onOpen(book.id)}
          onRemove={() => onRequestRemove(book)}
          onShowIndexStatus={() => onIndexStatus(book)}
        />
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
  onDeleteFailed,
  onRetry,
}: Pick<LibraryItemProps, 'book' | 'onDeleteFailed' | 'onRetry'>) {
  const message = useMessage();
  return (
    <span className="library-item__actions">
      <button type="button" onClick={() => onRetry(book.id)}>
        {message('library.retry')}
      </button>
      <button type="button" onClick={() => onDeleteFailed(book.id)}>
        {message('library.deleteFailed')}
      </button>
    </span>
  );
}
