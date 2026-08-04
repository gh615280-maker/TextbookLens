import type { BookSummary } from '../../lib/generated/book';

import { LibraryItem } from './LibraryItem';

interface LibraryGridProps {
  books: readonly BookSummary[];
  focusedBookId: string | null;
  selectedBookId: string | null;
  onDelete(bookId: string): void;
  onFocus(bookId: string): void;
  onMoveFocus(bookId: string, offset: -1 | 1): void;
  onOpen(bookId: string): void;
  onRetry(bookId: string): void;
  onSelect(bookId: string): void;
  label: string;
}

export function LibraryGrid({
  books,
  focusedBookId,
  selectedBookId,
  onDelete,
  onFocus,
  onMoveFocus,
  onOpen,
  onRetry,
  onSelect,
  label,
}: LibraryGridProps) {
  return (
    <div
      aria-label={label}
      className="library-grid"
      role="listbox"
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
          onDelete={onDelete}
          onFocus={onFocus}
          onMoveFocus={onMoveFocus}
          onOpen={onOpen}
          onRetry={onRetry}
          onSelect={onSelect}
        />
      ))}
    </div>
  );
}
