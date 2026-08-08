import { cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { ReaderSearch } from './ReaderSearch';

const labels = {
  region: 'Search this book',
  input: 'Search content',
  submit: 'Search',
  loading: 'Searching…',
  failed: 'Search failed.',
  navigateFailed: 'The original location could not be restored.',
  empty: 'No results.',
  page: (page: number) => `Page ${page}`,
  currentSection: 'Current section',
  scope: 'Search scope',
  all: 'All',
  book: 'Textbook only',
  question: 'Question descriptions only',
};

afterEach(() => cleanup());

describe('ReaderSearch', () => {
  it('searches all sources by default and supports each requested scope', async () => {
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
    render(
      <ReaderSearch
        bookId="book-1"
        format="pdf"
        search={search}
        onNavigate={vi.fn()}
        labels={labels}
      />,
    );
    await user.type(
      screen.getByRole('textbox', { name: labels.input }),
      'term',
    );
    await user.click(screen.getByRole('button', { name: labels.submit }));
    expect(search).toHaveBeenLastCalledWith('book-1', 'term', 50, 'all');
    expect(await screen.findByText('Page 7')).toBeVisible();

    await user.click(screen.getByRole('radio', { name: labels.question }));
    await user.click(screen.getByRole('button', { name: labels.submit }));
    expect(search).toHaveBeenLastCalledWith('book-1', 'term', 50, 'question');
  });

  it('drops stale results and exposes loading and empty states', async () => {
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
        labels={labels}
      />,
    );
    const input = within(container).getByRole('textbox', {
      name: labels.input,
    });
    await user.type(input, 'first');
    await user.click(
      within(container).getByRole('button', { name: labels.submit }),
    );
    expect(within(container).getByRole('status')).toBeVisible();
    await user.clear(input);
    await user.type(input, 'second');
    await user.click(
      within(container).getByRole('button', { name: labels.submit }),
    );
    resolveFirst([]);
    expect(await within(container).findByText(labels.empty)).toBeVisible();
  });

  it('reports a failed attempt to restore a result location', async () => {
    const user = userEvent.setup();
    const search = async () => [
      {
        snippet: 'question result',
        source: 'question' as const,
        locator: {
          format: 'pdf' as const,
          startPage: 2,
          endPage: 2,
          rectsByPage: null,
        },
        sectionTitle: null,
      },
    ];
    const onNavigate = async () => false;
    const { rerender } = render(
      <ReaderSearch
        bookId="book-1"
        format="pdf"
        search={search}
        onNavigate={onNavigate}
        labels={labels}
      />,
    );
    await user.type(screen.getByRole('textbox', { name: labels.input }), 'q');
    await user.click(screen.getByRole('button', { name: labels.submit }));
    await user.click(
      await screen.findByRole('button', { name: /question result/i }),
    );
    expect(await screen.findByRole('alert')).toHaveTextContent(
      labels.navigateFailed,
    );

    rerender(
      <ReaderSearch
        bookId="book-1"
        format="pdf"
        search={search}
        onNavigate={onNavigate}
        labels={{ ...labels, navigateFailed: '无法恢复原文位置。' }}
      />,
    );
    expect(screen.getByRole('alert')).toHaveTextContent('无法恢复原文位置。');
  });
});
