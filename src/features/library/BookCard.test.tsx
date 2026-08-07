import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { BookSummary } from '../../lib/generated/book';
import { BookCard } from './BookCard';

const BOOK_ID = '4f9a2c86-0da8-4dd4-a255-39b4cff89c66';

function book(overrides: Partial<BookSummary> = {}): BookSummary {
  return {
    id: BOOK_ID,
    title: '线性代数',
    originalFilename: 'linear-algebra.pdf',
    author: 'TextbookLens',
    language: 'zh-CN',
    format: 'pdf',
    importStatus: 'ready',
    importErrorCode: null,
    importErrorMessage: null,
    importErrorStage: null,
    readingProgress: 0.375,
    fullTextQaReady: false,
    indexAggregate: {
      status: 'not_required',
      totalPages: 0,
      indexedPages: 0,
      reviewPages: 0,
      failedPages: 0,
    },
    createdAt: '2026-08-01T00:00:00Z',
    updatedAt: '2026-08-01T00:00:00Z',
    lastOpenedAt: '2026-08-01T18:30:45Z',
    ...overrides,
  };
}

afterEach(cleanup);

describe('BookCard', () => {
  it('renders ready metadata and exposes an accessible open action', async () => {
    const user = userEvent.setup();
    const onOpen = vi.fn();
    render(
      <BookCard
        book={book()}
        onDelete={vi.fn()}
        onIndex={vi.fn()}
        onOpen={onOpen}
        onRetry={vi.fn()}
        timeFormat={{ locale: 'en-GB', timeZone: 'Asia/Shanghai' }}
      />,
    );

    expect(screen.getByRole('heading', { name: '线性代数' })).toBeVisible();
    expect(screen.getByText('PDF')).toBeVisible();
    expect(screen.getByText(/02 Aug 2026, 02:30/)).toBeVisible();
    expect(screen.getByRole('progressbar', { name: '阅读进度' })).toHaveValue(
      37.5,
    );
    await user.click(screen.getByRole('button', { name: '打开《线性代数》' }));
    expect(onOpen).toHaveBeenCalledWith(BOOK_ID);
  });

  it('shows safe failed details, filename, retry, and delete actions', async () => {
    const user = userEvent.setup();
    const onRetry = vi.fn();
    const onDelete = vi.fn();
    render(
      <BookCard
        book={book({
          importStatus: 'failed',
          importErrorCode: 'FILE_CORRUPTED',
          importErrorMessage: '文件已损坏或无法读取。',
          importErrorStage: 'parsing',
        })}
        onDelete={onDelete}
        onIndex={vi.fn()}
        onOpen={vi.fn()}
        onRetry={onRetry}
      />,
    );

    expect(screen.getByText('linear-algebra.pdf')).toBeVisible();
    expect(screen.getByText('解析失败')).toBeVisible();
    expect(screen.getByText('文件已损坏或无法读取。')).toBeVisible();
    await user.click(screen.getByRole('button', { name: '重新选择并重试' }));
    await user.click(screen.getByRole('button', { name: '删除失败记录' }));
    expect(onRetry).toHaveBeenCalledWith(BOOK_ID);
    expect(onDelete).toHaveBeenCalledWith(BOOK_ID);
  });

  it('never presents a recovered in-progress record as openable', () => {
    render(
      <BookCard
        book={book({ importStatus: 'indexing' })}
        onDelete={vi.fn()}
        onIndex={vi.fn()}
        onOpen={vi.fn()}
        onRetry={vi.fn()}
      />,
    );

    expect(screen.getByText('上次导入已中断')).toBeVisible();
    expect(
      screen.queryByRole('button', { name: /打开/ }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: '重新选择并重试' }),
    ).toBeEnabled();
  });

  it('offers AI-assisted indexing only for ready PDF books', async () => {
    const user = userEvent.setup();
    const onIndex = vi.fn();
    const view = render(
      <BookCard
        book={book()}
        onDelete={vi.fn()}
        onIndex={onIndex}
        onOpen={vi.fn()}
        onRetry={vi.fn()}
      />,
    );

    await user.click(screen.getByRole('button', { name: 'AI-assisted index' }));
    expect(onIndex).toHaveBeenCalledWith(BOOK_ID);

    view.rerender(
      <BookCard
        book={book({ format: 'epub' })}
        onDelete={vi.fn()}
        onIndex={onIndex}
        onOpen={vi.fn()}
        onRetry={vi.fn()}
      />,
    );
    expect(
      screen.queryByRole('button', { name: 'AI-assisted index' }),
    ).not.toBeInTheDocument();
  });
});
