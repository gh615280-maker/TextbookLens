import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { isSupportedSourcePath, LibraryDropTarget } from './LibraryDropTarget';

type DragPayload =
  | { type: 'enter'; paths: string[] }
  | { type: 'over'; paths?: never }
  | { type: 'drop'; paths: string[] }
  | { type: 'leave'; paths?: never };

const dragDropMock = vi.hoisted(() => {
  let listener: ((event: { payload: DragPayload }) => void) | undefined;
  const unlisten = vi.fn();
  const setListener = vi.fn(
    (next: (event: { payload: DragPayload }) => void) => {
      listener = next;
    },
  );
  return {
    emit: (payload: DragPayload) => listener?.({ payload }),
    unlisten,
    setListener,
  };
});

vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: async (
      listener: (event: { payload: DragPayload }) => void,
    ) => {
      dragDropMock.setListener(listener);
      return dragDropMock.unlisten;
    },
  }),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('LibraryDropTarget', () => {
  it('queues only supported native paths in received order and never accepts URL or text payloads', async () => {
    const onDrop = vi.fn();
    render(
      <LibraryDropTarget onDrop={onDrop}>
        <p>Library</p>
      </LibraryDropTarget>,
    );

    await waitFor(() => expect(dragDropMock.setListener).toHaveBeenCalled());
    act(() => dragDropMock.emit({ type: 'enter', paths: [] }));
    expect(
      screen.getByRole('status', {
        name: 'Drop supported textbook files to import',
      }),
    ).toBeVisible();

    act(() =>
      dragDropMock.emit({
        type: 'drop',
        paths: [
          'C:\\Books\\second.EPUB',
          'https://example.test/book.pdf',
          'C:\\Books\\notes.txt',
          'C:\\Books\\first.PDF',
          'not a path.docx',
        ],
      }),
    );
    expect(onDrop).toHaveBeenCalledWith([
      'C:\\Books\\second.EPUB',
      'C:\\Books\\first.PDF',
    ]);
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('clears nested drag state on leave and unmount', async () => {
    const { unmount } = render(
      <LibraryDropTarget onDrop={vi.fn()}>
        <p>Library</p>
      </LibraryDropTarget>,
    );

    await waitFor(() => expect(dragDropMock.setListener).toHaveBeenCalled());
    act(() => {
      dragDropMock.emit({ type: 'enter', paths: [] });
      dragDropMock.emit({ type: 'enter', paths: [] });
      dragDropMock.emit({ type: 'leave' });
    });
    expect(screen.getByRole('status')).toBeVisible();
    act(() => dragDropMock.emit({ type: 'leave' }));
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    unmount();
    expect(dragDropMock.unlisten).toHaveBeenCalledOnce();
  });
});

describe('isSupportedSourcePath', () => {
  it.each([
    ['C:\\Books\\book.pdf', true],
    ['C:\\Books\\book.epub', true],
    ['C:\\Books\\book.docx', true],
    ['https://example.test/book.pdf', false],
    ['plain text.pdf', false],
    ['C:\\Books\\book.txt', false],
  ])('accepts %s only when appropriate', (path, expected) => {
    expect(isSupportedSourcePath(path)).toBe(expected);
  });
});
