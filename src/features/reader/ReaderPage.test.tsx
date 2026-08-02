import { render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { ReaderPage } from './ReaderPage';

const state = vi.hoisted(() => ({
  pdfOpen: vi.fn(async () => {}),
  pdfDispose: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (command: string) => {
    if (command === 'get_reader_settings')
      return {
        fontScale: 1,
        lineHeight: 1.5,
        readerWidth: 72,
        pdfZoom: 1,
        theme: 'system',
      };
    if (command === 'get_reader_bootstrap')
      return {
        book: {
          id: 'book-1',
          title: 'Fixture',
          originalFilename: 'fixture.pdf',
          author: null,
          language: null,
          format: 'pdf',
          importStatus: 'ready',
          importErrorCode: null,
          importErrorMessage: null,
          importErrorStage: null,
          readingProgress: 0,
          createdAt: '2026-08-02T00:00:00Z',
          updatedAt: '2026-08-02T00:00:00Z',
          lastOpenedAt: null,
        },
        lastLocator: null,
      };
    if (command === 'read_book_source') return new Uint8Array([1, 2, 3]);
    if (
      command === 'list_reader_sections' ||
      command === 'list_annotation_markers'
    )
      return [];
    throw new Error(`Unexpected command: ${command}`);
  }),
}));

vi.mock('./pdf/PdfReaderAdapter', () => ({
  PdfReaderAdapter: class {
    readonly format = 'pdf';
    open = state.pdfOpen;
    dispose = state.pdfDispose;
    getSelectionSnapshot = () => null;
    navigate = async () => ({ found: true });
    showAnnotations = async () => [];
    search = async () => [];
    getProgress = () => ({ fraction: 0, locator: null });
  },
}));

vi.mock('./epub/EpubReaderAdapter', () => ({
  EpubReaderAdapter: class {},
}));

vi.mock('./docx/DocxReaderAdapter', () => ({
  DocxReaderAdapter: class {},
}));

afterEach(() => {
  vi.clearAllMocks();
});

describe('ReaderPage', () => {
  it('opens the format adapter in the mounted document region and disposes it', async () => {
    const view = render(
      <MemoryRouter initialEntries={['/reader/book-1']}>
        <Routes>
          <Route path="/reader/:bookId" element={<ReaderPage />} />
        </Routes>
      </MemoryRouter>,
    );

    expect(screen.getByRole('region', { name: '阅读文档' })).toBeVisible();
    expect(
      screen.getByRole('complementary', { name: '标记历史' }),
    ).toBeVisible();
    await waitFor(() => expect(state.pdfOpen).toHaveBeenCalledOnce());

    view.unmount();
    expect(state.pdfDispose).toHaveBeenCalledOnce();
  });
});
