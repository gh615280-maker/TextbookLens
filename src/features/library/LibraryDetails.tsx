import { useMessage } from '../../app/LanguageProvider';
import type { BookSummary } from '../../lib/generated/book';

import { LibraryItem } from './LibraryItem';

interface LibraryDetailsProps {
  books: readonly BookSummary[];
  focusedBookId: string | null;
  selectedBookId: string | null;
  onIndexStatus(book: BookSummary): void;
  onStartIndex(book: BookSummary): void;
  onDeleteFailed(bookId: string): void;
  onFocus(bookId: string): void;
  onMoveFocus(bookId: string, offset: -1 | 1): void;
  onOpen(bookId: string): void;
  onRetry(bookId: string): void;
  onRequestRemove(book: BookSummary): void;
  onSelect(bookId: string): void;
}

export function LibraryDetails({
  books,
  focusedBookId,
  selectedBookId,
  onIndexStatus,
  onStartIndex,
  onDeleteFailed,
  onFocus,
  onMoveFocus,
  onOpen,
  onRetry,
  onRequestRemove,
  onSelect,
}: LibraryDetailsProps) {
  const message = useMessage();
  return (
    <div
      aria-label={message('library.itemsLabel')}
      className="library-details"
      role="grid"
      style={{ overflowX: 'auto' }}
    >
      <div role="row" className="library-details__header">
        <span role="columnheader">{message('library.column.title')}</span>
        <span role="columnheader">{message('library.column.format')}</span>
        <span role="columnheader">
          {message('library.column.importProgress')}
        </span>
        <span role="columnheader">{message('library.column.indexStatus')}</span>
        <span role="columnheader">{message('library.column.lastOpened')}</span>
        <span role="columnheader">{message('library.column.actions')}</span>
      </div>
      {books.map((book, index) => (
        <LibraryItem
          key={book.id}
          book={book}
          focused={focusedBookId === book.id || (!focusedBookId && index === 0)}
          selected={selectedBookId === book.id}
          view="details"
          onIndexStatus={onIndexStatus}
          onStartIndex={onStartIndex}
          onDeleteFailed={onDeleteFailed}
          onFocus={onFocus}
          onMoveFocus={onMoveFocus}
          onOpen={onOpen}
          onRetry={onRetry}
          onRequestRemove={onRequestRemove}
          onSelect={onSelect}
        />
      ))}
    </div>
  );
}
