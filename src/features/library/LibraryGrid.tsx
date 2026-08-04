import type { BookSummary } from '../../lib/generated/book';

import { LibraryItem } from './LibraryItem';

interface LibraryGridProps {
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
  label: string;
}

export function LibraryGrid({
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
  label,
}: LibraryGridProps) {
  return (
    <ul
      aria-label={label}
      className="library-grid"
      style={{
        display: 'grid',
        gap: 'var(--space-3)',
        gridTemplateColumns: 'repeat(auto-fit, minmax(min(100%, 12rem), 1fr))',
      }}
    >
      {books.map((book, index) => (
        <LibraryItem
          key={book.id}
          book={book}
          focused={focusedBookId === book.id || (!focusedBookId && index === 0)}
          selected={selectedBookId === book.id}
          view="large"
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
    </ul>
  );
}
