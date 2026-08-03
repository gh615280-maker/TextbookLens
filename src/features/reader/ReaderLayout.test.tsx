import { createRef } from 'react';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { ReaderSettings } from './api';
import { ReaderLayout } from './ReaderLayout';

const settings: ReaderSettings = {
  fontScale: 1,
  lineHeight: 1.6,
  readerWidth: 72,
  pdfZoom: 1,
  theme: 'system',
};

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  delete (HTMLElement.prototype as Partial<HTMLElement>).requestFullscreen;
  delete (document as Partial<Document>).exitFullscreen;
});

describe('ReaderLayout', () => {
  it('renders exactly the minimal toolbar contract and keeps the adapter mount stable across drawers', async () => {
    const user = userEvent.setup();
    const readerContainerRef = createRef<HTMLDivElement>();
    const markerHistoryRef = createRef<HTMLDivElement>();

    render(
      <ReaderLayout
        title="Fixture"
        location="第 3 页"
        bookId="book-1"
        readerContainerRef={readerContainerRef}
        markerHistoryRef={markerHistoryRef}
        settings={settings}
        onSettingsChange={() => {}}
        search={async () => []}
      />,
    );

    const toolbar = screen.getByRole('toolbar', { name: '阅读工具栏' });
    expect(toolbar).toHaveTextContent('书库');
    expect(toolbar).toHaveTextContent('目录');
    expect(toolbar).toHaveTextContent('Fixture · 第 3 页');
    expect(toolbar).toHaveTextContent('搜索');
    expect(toolbar).toHaveTextContent('框选');
    expect(toolbar).toHaveTextContent('Aa');
    expect(screen.getByRole('button', { name: /框选/ })).toBeDisabled();
    expect(screen.getByRole('button', { name: '进入全屏' })).toBeEnabled();

    const documentRoot = readerContainerRef.current;
    await user.click(screen.getByRole('button', { name: '目录' }));
    expect(screen.getByRole('dialog', { name: '教材目录' })).toBeVisible();
    await user.click(screen.getByRole('button', { name: '关闭' }));
    await user.click(screen.getByRole('button', { name: '阅读设置' }));
    expect(screen.getByRole('dialog', { name: '阅读设置' })).toBeVisible();
    expect(readerContainerRef.current).toBe(documentRoot);
    expect(markerHistoryRef.current?.isConnected).toBe(true);
  });

  it('closes the top transient layer on Escape and returns focus to its trigger', async () => {
    const user = userEvent.setup();
    render(
      <ReaderLayout title="Fixture" bookId="book-1" search={async () => []} />,
    );

    const searchButton = screen.getByRole('button', { name: '搜索' });
    await user.click(searchButton);
    expect(screen.getByRole('dialog', { name: '书内搜索' })).toBeVisible();
    await user.keyboard('{Escape}');
    expect(
      screen.queryByRole('dialog', { name: '书内搜索' }),
    ).not.toBeInTheDocument();
    expect(searchButton).toHaveFocus();
  });

  it('keeps toolbar and F11 fullscreen state synchronized', async () => {
    let fullscreenElement: Element | null = null;
    Object.defineProperty(document, 'fullscreenElement', {
      configurable: true,
      get: () => fullscreenElement,
    });
    Object.defineProperty(HTMLElement.prototype, 'requestFullscreen', {
      configurable: true,
      value: vi.fn(async () => {
        fullscreenElement = document.querySelector('.reader-layout');
        document.dispatchEvent(new Event('fullscreenchange'));
      }),
    });
    Object.defineProperty(document, 'exitFullscreen', {
      configurable: true,
      value: vi.fn(async () => {
        fullscreenElement = null;
        document.dispatchEvent(new Event('fullscreenchange'));
      }),
    });
    render(<ReaderLayout title="Fixture" />);

    fireEvent.keyDown(window, { key: 'F11' });
    expect(
      await screen.findByRole('button', { name: '退出全屏' }),
    ).toHaveAttribute('aria-pressed', 'true');
    await userEvent.click(screen.getByRole('button', { name: '退出全屏' }));
    expect(
      await screen.findByRole('button', { name: '进入全屏' }),
    ).toHaveAttribute('aria-pressed', 'false');
  });

  it('shows and completes the first-reader hint only when requested', async () => {
    const user = userEvent.setup();
    const onComplete = vi.fn();
    const view = render(
      <ReaderLayout
        title="Fixture"
        firstHintVisible
        onCompleteFirstHint={onComplete}
      />,
    );

    expect(screen.getByText('阅读提示')).toBeVisible();
    await user.click(screen.getByRole('button', { name: '知道了' }));
    expect(onComplete).toHaveBeenCalledOnce();

    view.rerender(<ReaderLayout title="Fixture" firstHintVisible={false} />);
    expect(screen.queryByText('阅读提示')).not.toBeInTheDocument();
  });
});
