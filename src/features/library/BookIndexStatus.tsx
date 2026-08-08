import { Link } from 'react-router-dom';

import { useMessage } from '../../app/LanguageProvider';
import type { BookSummary } from '../../lib/generated/book';

interface BookIndexStatusProps {
  book: BookSummary;
  onOpenStatus(book: BookSummary): void;
  onStartIndex(book: BookSummary): void;
}

export function BookIndexStatus({
  book,
  onOpenStatus,
  onStartIndex,
}: BookIndexStatusProps) {
  const message = useMessage();
  const aggregate = book.indexAggregate;
  const unavailable = aggregate.status === 'not_required';
  const label =
    aggregate.status === 'ready'
      ? message('library.indexCompleted')
      : message(`library.indexStatus.${aggregate.status}`);
  const canStart = book.importStatus === 'ready' && book.format === 'pdf';

  if (canStart) {
    return (
      <span className="library-index-actions">
        {book.fullTextQaReady ? (
          <>
            <span role="status">{message('library.fullTextQaReady')}</span>
            <Link
              to={`/books/${encodeURIComponent(book.id)}/overview`}
              aria-label={message('library.fullTextQaAskForBook', {
                title: book.title,
              })}
            >
              {message('library.fullTextQaAsk')}
            </Link>
          </>
        ) : null}
        <button type="button" onClick={() => onStartIndex(book)}>
          {message('library.indexStart')}
        </button>
        {!unavailable ? (
          <IndexStatusButton
            book={book}
            label={label}
            onOpenStatus={onOpenStatus}
          />
        ) : null}
      </span>
    );
  }
  return unavailable ? (
    <button type="button" disabled>
      {label}
    </button>
  ) : (
    <IndexStatusButton book={book} label={label} onOpenStatus={onOpenStatus} />
  );
}

function IndexStatusButton({
  book,
  label,
  onOpenStatus,
}: {
  book: BookSummary;
  label: string;
  onOpenStatus(book: BookSummary): void;
}) {
  const message = useMessage();
  const aggregate = book.indexAggregate;
  return (
    <button
      aria-label={message('library.indexStatus.action', { title: book.title })}
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
