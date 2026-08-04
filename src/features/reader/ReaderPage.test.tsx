import { useState, type ReactNode } from 'react';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { LanguageContext } from '../../app/LanguageProvider';
import { formatMessage, type UiLanguage } from '../../lib/i18n';
import { ReaderPage } from './ReaderPage';

const state = vi.hoisted(() => ({
  pdfOpen: vi.fn(async () => {}),
  pdfDispose: vi.fn(),
  commands: [] as string[],
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (command: string) => {
    state.commands.push(command);
    if (
      command === 'get_app_settings' ||
      command === 'complete_first_reader_hint'
    )
      return {
        onboardingCompleted: false,
        activeProviderProfileId: null,
        theme: 'system',
        contextMode: 'standard',
        uiLanguage: 'zh-CN',
        uiLanguageInitialized: true,
        firstReaderHintCompleted: command === 'complete_first_reader_hint',
      };
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
    if (command === 'update_reader_settings')
      return {
        fontScale: 1,
        lineHeight: 1.5,
        readerWidth: 72,
        pdfZoom: 1.1,
        theme: 'system',
      };
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
  cleanup();
  vi.clearAllMocks();
  state.commands.length = 0;
});

describe('ReaderPage', () => {
  it('shows the stable local-reading limitation after rejecting AI indexing', () => {
    render(
      <LanguageHarness>
        <MemoryRouter initialEntries={['/reader/book-1?index=local-only']}>
          <Routes>
            <Route path="/reader/:bookId" element={<ReaderPage />} />
          </Routes>
        </MemoryRouter>
      </LanguageHarness>,
    );

    expect(
      screen.getByRole('status', {
        name: '',
      }),
    ).toHaveTextContent(
      'This PDF has unreliable or no local text. Search and text-based AI are limited; local page reading remains available.',
    );
  });

  it('opens the format adapter in the mounted document region and disposes it', async () => {
    const view = render(
      <LanguageHarness>
        <MemoryRouter initialEntries={['/reader/book-1']}>
          <Routes>
            <Route path="/reader/:bookId" element={<ReaderPage />} />
          </Routes>
        </MemoryRouter>
      </LanguageHarness>,
    );

    expect(screen.getByRole('region', { name: '阅读文档' })).toBeVisible();
    expect(
      screen.getByRole('complementary', { name: '标记历史' }),
    ).toBeVisible();
    await waitFor(() => expect(state.pdfOpen).toHaveBeenCalledOnce());

    await userEvent.click(screen.getByRole('button', { name: '阅读设置' }));
    expect(screen.getByRole('dialog', { name: '阅读设置' })).toBeVisible();
    expect(state.pdfDispose).not.toHaveBeenCalled();

    await waitFor(() => expect(screen.getByText('阅读提示')).toBeVisible());
    await userEvent.click(screen.getByRole('button', { name: '知道了' }));
    await waitFor(() =>
      expect(state.commands).toContain('complete_first_reader_hint'),
    );
    expect(state.pdfDispose).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole('button', { name: '切换壳语言' }));
    expect(
      screen.getByRole('toolbar', { name: 'Reader toolbar' }),
    ).toBeVisible();
    expect(state.pdfOpen).toHaveBeenCalledOnce();
    expect(state.pdfDispose).not.toHaveBeenCalled();

    view.unmount();
    expect(state.pdfDispose).toHaveBeenCalledOnce();
  });
});

function LanguageHarness({ children }: { children: ReactNode }) {
  const [uiLanguage, setUiLanguage] = useState<UiLanguage>('zh-CN');
  return (
    <LanguageContext.Provider
      value={{
        uiLanguage,
        isLoading: false,
        statusMessage: null,
        switchLanguage: async (language) => setUiLanguage(language),
        message: (key, values) => formatMessage(uiLanguage, key, values),
      }}
    >
      <button type="button" onClick={() => setUiLanguage('en')}>
        切换壳语言
      </button>
      {children}
    </LanguageContext.Provider>
  );
}
