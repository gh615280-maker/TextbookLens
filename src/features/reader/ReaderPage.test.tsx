import { useState, type ReactNode } from 'react';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { LanguageContext } from '../../app/LanguageProvider';
import { formatMessage, type UiLanguage } from '../../lib/i18n';
import { LearningRequestProvider } from '../learning/LearningRequestProvider';
import { LearningRequestStore } from '../learning/learning-request-store';
import { ReaderPage } from './ReaderPage';

const state = vi.hoisted(() => ({
  pdfOpen: vi.fn(async () => {}),
  pdfDispose: vi.fn(),
  pdfShowAnnotations: vi.fn(async () => []),
  annotationMarkers: [] as unknown[],
  commands: [] as string[],
  providerOperations: [] as string[],
  textProfileId: '33333333-3333-4333-8333-333333333333',
  visionProfileId: '44444444-4444-4444-8444-444444444444',
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (command: string, args?: { operation?: string }) => {
    state.commands.push(command);
    if (
      command === 'get_app_settings' ||
      command === 'complete_first_reader_hint'
    )
      return {
        onboardingCompleted: false,
        activeProviderProfileId: state.textProfileId,
        defaultLearningProfileId: state.textProfileId,
        defaultVisionProfileId: state.visionProfileId,
        theme: 'system',
        contextMode: 'standard',
        uiLanguage: 'zh-CN',
        uiLanguageInitialized: true,
        firstReaderHintCompleted: command === 'complete_first_reader_hint',
      };
    if (command === 'list_provider_profiles') {
      const operation = args?.operation ?? '';
      state.providerOperations.push(operation);
      const vision = operation === 'vision_learning';
      return [
        {
          id: vision ? state.visionProfileId : state.textProfileId,
          kind: vision ? 'kimi' : 'deepseek',
          displayName: vision ? 'Kimi Vision' : 'DeepSeek Text',
          modelId: vision ? 'kimi-k3' : 'deepseek-v4-flash',
          contextWindowTokens: 131072,
          isActive: !vision,
          credentialStatus: 'available',
          validatedAt: '2026-08-07T00:00:00Z',
        },
      ];
    }
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
          indexAggregate: {
            status: 'not_required',
            totalPages: 0,
            indexedPages: 0,
            reviewPages: 0,
            failedPages: 0,
          },
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
    if (command === 'list_reader_sections') return [];
    if (command === 'list_annotation_markers') return state.annotationMarkers;
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
    showAnnotations = state.pdfShowAnnotations;
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
  state.annotationMarkers.length = 0;
  state.commands.length = 0;
  state.providerOperations.length = 0;
});

describe('ReaderPage', () => {
  it('loads separate defaults for text and visual learning', async () => {
    render(
      <LanguageHarness>
        <MemoryRouter initialEntries={['/reader/book-1']}>
          <Routes>
            <Route path="/reader/:bookId" element={<ReaderPage />} />
          </Routes>
        </MemoryRouter>
      </LanguageHarness>,
    );

    await waitFor(() =>
      expect(state.providerOperations).toEqual([
        'text_learning',
        'vision_learning',
      ]),
    );
  });

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

  it('refreshes persisted highlights when a selection request completes', async () => {
    const store = new LearningRequestStore();
    render(
      <LearningRequestProvider store={store}>
        <LanguageHarness>
          <MemoryRouter initialEntries={['/reader/book-1']}>
            <Routes>
              <Route path="/reader/:bookId" element={<ReaderPage />} />
            </Routes>
          </MemoryRouter>
        </LanguageHarness>
      </LearningRequestProvider>,
    );
    await waitFor(() =>
      expect(state.pdfShowAnnotations).toHaveBeenCalledOnce(),
    );

    const requestId = '11111111-1111-4111-8111-111111111111';
    store.applySnapshot({
      requestId,
      conversationId: null,
      status: 'preparing',
      text: '',
      usage: null,
      safeError: null,
      lastSeq: 0,
    });
    store.setPresentation(requestId, {
      action: 'explain',
      selectionLabel: 'Selected region',
      provider: 'Kimi',
      model: 'moonshot-v1-8k',
    });
    state.annotationMarkers.push({
      id: 'marker-1',
      kind: 'ai_conversation',
      conversationId: '22222222-2222-4222-8222-222222222222',
      label: 'View AI conversation marker',
      relocationStatus: 'primary',
      anchor: {
        kind: 'region',
        region: {
          locator: { format: 'pdf', page: 1 },
          rect: { x: 0.1, y: 0.1, width: 0.2, height: 0.1 },
          contentSha256: 'f'.repeat(64),
          textFallback: null,
        },
      },
    });
    store.applyEvent({
      requestId,
      seq: 1,
      event: { type: 'text_delta', text: 'Answer' },
    });
    store.applyEvent({
      requestId,
      seq: 2,
      event: {
        type: 'completed',
        conversationId: '22222222-2222-4222-8222-222222222222',
      },
    });

    await waitFor(() =>
      expect(state.pdfShowAnnotations).toHaveBeenCalledTimes(2),
    );
    expect(state.pdfShowAnnotations).toHaveBeenLastCalledWith([
      expect.objectContaining({ id: 'marker-1' }),
    ]);
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
