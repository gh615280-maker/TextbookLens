import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { ReaderSearch } from './ReaderSearch';

describe('ReaderSearch', () => {
  it('never searches blank queries or without a book ID, caps calls, and labels PDF citations', async () => {
    const user = userEvent.setup();
    const search = vi.fn(async () => [
      {
        snippet: 'match',
        locator: {
          format: 'pdf' as const,
          startPage: 7,
          endPage: 7,
          rectsByPage: null,
        },
        sectionTitle: null,
      },
    ]);
    const { rerender } = render(
      <ReaderSearch
        bookId={null}
        format="pdf"
        search={search}
        onNavigate={vi.fn()}
      />,
    );
    await user.click(screen.getByRole('button', { name: '搜索' }));
    expect(search).not.toHaveBeenCalled();
    rerender(
      <ReaderSearch
        bookId="book-1"
        format="pdf"
        search={search}
        onNavigate={vi.fn()}
      />,
    );
    await user.type(
      screen.getByRole('textbox', { name: '搜索书内内容' }),
      'term',
    );
    await user.click(screen.getByRole('button', { name: '搜索' }));
    expect(search).toHaveBeenCalledWith('book-1', 'term', 50);
    expect(await screen.findByText('第 7 页')).toBeVisible();
  });
  it('drops stale results and exposes loading and error states', async () => {
    const user = userEvent.setup();
    let resolveFirst!: (value: never[]) => void;
    const search = vi
      .fn()
      .mockImplementationOnce(
        () =>
          new Promise<never[]>((resolve) => {
            resolveFirst = resolve;
          }),
      )
      .mockResolvedValueOnce([]);
    const { container } = render(
      <ReaderSearch
        bookId="book-1"
        format="epub"
        search={search}
        onNavigate={vi.fn()}
      />,
    );
    const input = within(container).getByRole('textbox', {
      name: '搜索书内内容',
    });
    await user.type(input, 'first');
    await user.click(within(container).getByRole('button', { name: '搜索' }));
    expect(within(container).getByRole('status')).toBeVisible();
    await user.clear(input);
    await user.type(input, 'second');
    await user.click(within(container).getByRole('button', { name: '搜索' }));
    resolveFirst([]);
    expect(await within(container).findByText('没有搜索结果。')).toBeVisible();
  });
});
