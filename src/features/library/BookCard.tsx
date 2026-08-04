import type { BookSummary, ImportErrorStage } from '../../lib/generated/book';
import { formatPersistedUtc, type TimeFormatOptions } from '../../lib/time';

interface BookCardProps {
  book: BookSummary;
  onOpen(bookId: string): void;
  onIndex(bookId: string): void;
  onRetry(bookId: string): void;
  onDelete(bookId: string): void;
  indexLabel?: string;
  timeFormat?: TimeFormatOptions;
}

const stageLabels: Record<ImportErrorStage, string> = {
  copying: '复制',
  parsing: '解析',
  indexing: '索引',
};

export function BookCard({
  book,
  onOpen,
  onIndex,
  onRetry,
  onDelete,
  indexLabel = 'AI-assisted index',
  timeFormat,
}: BookCardProps) {
  const ready = book.importStatus === 'ready';
  const failed = book.importStatus === 'failed';
  const lastOpened = book.lastOpenedAt
    ? formatPersistedUtc(book.lastOpenedAt, timeFormat)
    : '尚未打开';

  return (
    <article aria-labelledby={`book-${book.id}-title`} className="book-card">
      <div className="book-card__cover" aria-hidden="true">
        <span>📘</span>
      </div>
      <div className="book-card__body">
        <h2 id={`book-${book.id}-title`}>{book.title}</h2>
        <p className="book-card__format">{book.format.toUpperCase()}</p>
        {ready ? (
          <>
            <p>最后阅读：{lastOpened}</p>
            <label className="book-card__progress">
              <span>阅读进度</span>
              <progress
                aria-label="阅读进度"
                max={100}
                value={book.readingProgress * 100}
              />
              <span>{Math.round(book.readingProgress * 100)}%</span>
            </label>
            <button type="button" onClick={() => onOpen(book.id)}>
              打开《{book.title}》
            </button>
            {book.format === 'pdf' ? (
              <button type="button" onClick={() => onIndex(book.id)}>
                {indexLabel}
              </button>
            ) : null}
          </>
        ) : (
          <div
            className="book-card__failure"
            role="group"
            aria-label="导入失败操作"
          >
            <p>{book.originalFilename}</p>
            <p className="status-label">
              {failed && book.importErrorStage
                ? `${stageLabels[book.importErrorStage]}失败`
                : '上次导入已中断'}
            </p>
            <p>
              {failed
                ? (book.importErrorMessage ?? '导入未能安全完成。')
                : '上次导入已中断，请重新选择原文件。'}
            </p>
            <div className="button-row">
              <button type="button" onClick={() => onRetry(book.id)}>
                重新选择并重试
              </button>
              <button type="button" onClick={() => onDelete(book.id)}>
                删除失败记录
              </button>
            </div>
          </div>
        )}
      </div>
    </article>
  );
}
