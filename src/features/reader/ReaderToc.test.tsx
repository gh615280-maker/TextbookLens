import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { ReaderToc } from './ReaderToc';

const sections = [
  {
    id: 'one',
    parentId: null,
    ordinal: 0,
    title: '第一章',
    locator: {
      format: 'pdf' as const,
      startPage: 1,
      endPage: 1,
      rectsByPage: null,
    },
  },
  {
    id: 'two',
    parentId: 'one',
    ordinal: 1,
    title: '第一节',
    locator: {
      format: 'pdf' as const,
      startPage: 2,
      endPage: 2,
      rectsByPage: null,
    },
  },
];
describe('ReaderToc', () => {
  it('builds a stable nested TOC, marks the current item, and activates navigation by keyboard', async () => {
    const user = userEvent.setup();
    const navigate = vi.fn(async () => true);
    render(
      <ReaderToc
        sections={sections}
        currentSectionId="two"
        onNavigate={navigate}
      />,
    );
    expect(screen.getByRole('button', { name: '第一节' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    await user.click(screen.getByRole('button', { name: 'Collapse 第一章' }));
    expect(
      screen.queryByRole('button', { name: '第一节' }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Expand 第一章' }));
    screen.getByRole('button', { name: '第一节' }).focus();
    await user.keyboard('{Enter}');
    expect(navigate).toHaveBeenCalledWith(sections[1]!.locator);
  });
});
