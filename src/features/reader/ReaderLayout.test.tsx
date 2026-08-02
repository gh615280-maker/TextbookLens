import { createRef } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import { ReaderLayout } from './ReaderLayout';

describe('ReaderLayout', () => {
  it('mounts the document adapter and marker history in separate accessible regions', () => {
    const readerContainerRef = createRef<HTMLDivElement>();
    const markerHistoryRef = createRef<HTMLDivElement>();

    const { unmount } = render(
      <ReaderLayout
        title="Fixture"
        readerContainerRef={readerContainerRef}
        markerHistoryRef={markerHistoryRef}
      />,
    );

    expect(screen.getByRole('region', { name: '阅读文档' })).toBe(
      readerContainerRef.current,
    );
    expect(
      screen.getByRole('complementary', { name: '标记历史' }),
    ).toContainElement(markerHistoryRef.current);
    unmount();
  });

  it('provides collapsible TOC and panel regions with accessible toolbar controls', async () => {
    const user = userEvent.setup();
    render(<ReaderLayout title="Fixture" />);

    expect(screen.getByRole('main', { name: '阅读内容' })).toBeVisible();
    expect(screen.getByRole('complementary', { name: '目录' })).toBeVisible();
    expect(
      screen.getByRole('complementary', { name: '学习面板' }),
    ).toBeVisible();
    await user.click(screen.getByRole('button', { name: '折叠目录' }));
    await user.click(screen.getByRole('button', { name: '折叠学习面板' }));
    expect(
      screen.queryByRole('complementary', { name: '目录' }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole('complementary', { name: '学习面板' }),
    ).not.toBeInTheDocument();
  });

  it('clears transient selection UI on Escape without moving focus for panel updates', async () => {
    const user = userEvent.setup();
    render(
      <ReaderLayout
        title="Fixture"
        initialSelectionActive
        panelContent="准备中"
      />,
    );
    const toggle = screen.getByRole('button', { name: '折叠目录' });
    toggle.focus();

    await user.keyboard('{Escape}');

    expect(
      screen.queryByRole('status', { name: '已选择文本' }),
    ).not.toBeInTheDocument();
    expect(document.activeElement).toBe(toggle);
  });
});
