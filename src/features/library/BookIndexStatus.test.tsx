import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MemoryRouter } from 'react-router-dom';

import { LanguageContext } from '../../app/LanguageProvider';
import type { BookSummary } from '../../lib/generated/book';
import { formatMessage } from '../../lib/i18n';
import { BookIndexStatus } from './BookIndexStatus';

const book: BookSummary = {
  id: '4f9a2c86-0da8-4dd4-a255-39b4cff89c66',
  title: '线性代数',
  originalFilename: 'linear-algebra.pdf',
  author: null,
  language: 'zh-CN',
  format: 'pdf',
  importStatus: 'ready',
  importErrorCode: null,
  importErrorMessage: null,
  importErrorStage: null,
  readingProgress: 0,
  fullTextQaReady: true,
  indexAggregate: {
    status: 'ready',
    totalPages: 3,
    indexedPages: 3,
    reviewPages: 0,
    failedPages: 0,
  },
  createdAt: '2026-08-01T00:00:00Z',
  updatedAt: '2026-08-01T00:00:00Z',
  lastOpenedAt: null,
};

afterEach(cleanup);

describe('BookIndexStatus', () => {
  it('keeps reindex available and shows completion directly underneath', async () => {
    const user = userEvent.setup();
    const onStartIndex = vi.fn();
    const onOpenStatus = vi.fn();
    render(
      <MemoryRouter>
        <LanguageContext.Provider
          value={{
            uiLanguage: 'zh-CN',
            isLoading: false,
            statusMessage: null,
            switchLanguage: vi.fn(),
            message: (key, values) => formatMessage('zh-CN', key, values),
          }}
        >
          <BookIndexStatus
            book={book}
            onOpenStatus={onOpenStatus}
            onStartIndex={onStartIndex}
          />
        </LanguageContext.Provider>
      </MemoryRouter>,
    );

    const start = screen.getByRole('button', { name: '准备全文问答' });
    expect(screen.getByRole('status')).toHaveTextContent('全文问答已就绪');
    expect(
      screen.getByRole('link', { name: '就《线性代数》进行全文提问' }),
    ).toHaveAttribute('href', `/books/${book.id}/overview`);
    expect(start.parentElement).toHaveTextContent('准备全文问答已完成');
    await user.click(start);
    expect(onStartIndex).toHaveBeenCalledWith(book);

    await user.click(
      screen.getByRole('button', { name: '查看《线性代数》的索引状态' }),
    );
    expect(onOpenStatus).toHaveBeenCalledWith(book);
  });
});
