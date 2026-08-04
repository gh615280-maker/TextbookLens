import { useMessage } from '../../app/LanguageProvider';
import type { BookSummary } from '../../lib/generated/book';

interface BookIndexStatusProps {
  book: BookSummary;
  onOpenStatus(book: BookSummary): void;
}

export function BookIndexStatus({ book, onOpenStatus }: BookIndexStatusProps) {
  const message = useMessage();
  const aggregate = book.indexAggregate;
  const unavailable = aggregate.status === 'not_required';
  const label = message(`library.indexStatus.${aggregate.status}`);

  return (
    <button
      aria-label={message('library.indexStatus.action', { title: book.title })}
      disabled={unavailable}
      type="button"
      onClick={() => onOpenStatus(book)}
    >
      <span>{label}</span>
      {aggregate.totalPages > 0 ? (
        <span>
          {' '}
          {message('library.indexStatus.counts', {
            failed: aggregate.failedPages,
            indexed: aggregate.indexedPages,
            review: aggregate.reviewPages,
            total: aggregate.totalPages,
          })}
        </span>
      ) : null}
    </button>
  );
}
