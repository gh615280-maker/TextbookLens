import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { LanguageProvider } from '../../app/LanguageProvider';
import type { BookSummary } from '../../lib/generated/book';
import type { AppSettingsDto } from '../../lib/generated/settings';
import type { UserFacingError } from '../../lib/errors';
import type { LibraryApi } from '../library/api';
import { LibraryPage } from '../library/LibraryPage';
import type { ImportCoordinatorPort } from './ImportCoordinator';
import type { ImportEvent } from './parser-contract';

const { openMock } = vi.hoisted(() => ({ openMock: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: openMock }));

const BOOK_ID = '4f9a2c86-0da8-4dd4-a255-39b4cff89c66';
const EXISTING_ID = '70c92c3b-d44a-5345-a7eb-839e79c5b322';
const languageSettings: AppSettingsDto = {
  onboardingCompleted: true,
  activeProviderProfileId: null,
  defaultLearningProfileId: null,
  defaultVisionProfileId: null,
  theme: 'system',
  contextMode: 'standard',
  uiLanguage: 'zh-CN',
  uiLanguageInitialized: true,
  firstReaderHintCompleted: true,
};

function book(overrides: Partial<BookSummary> = {}): BookSummary {
  return {
    id: BOOK_ID,
    title: 'Fixture',
    originalFilename: 'fixture.pdf',
    author: null,
    language: 'zh-CN',
    format: 'pdf',
    importStatus: 'ready',
    importErrorCode: null,
    importErrorMessage: null,
    importErrorStage: null,
    readingProgress: 0,
    fullTextQaReady: false,
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
    ...overrides,
  };
}

class FakeLibraryApi implements LibraryApi {
  books: BookSummary[] = [];
  readonly deleteFailedImport = vi.fn(async (bookId: string) => {
    this.books = this.books.filter((candidate) => candidate.id !== bookId);
  });

  async listBooks(): Promise<BookSummary[]> {
    return this.books;
  }
}

class FakeCoordinator implements ImportCoordinatorPort {
  importImpl: ImportCoordinatorPort['importDocument'] = async () => book();
  retryImpl: ImportCoordinatorPort['retryDocument'] = async () => book();
  readonly importDocument = vi.fn<ImportCoordinatorPort['importDocument']>(
    (...args) => this.importImpl(...args),
  );
  readonly retryDocument = vi.fn<ImportCoordinatorPort['retryDocument']>(
    (...args) => this.retryImpl(...args),
  );
  readonly cancel = vi.fn(async () => {});
  readonly cancelPending = vi.fn();
}

function LocationProbe() {
  return <output data-testid="location">{useLocation().pathname}</output>;
}

function renderFlow(api: LibraryApi, coordinator: ImportCoordinatorPort) {
  const languageApi = {
    getAppSettings: async () => languageSettings,
    initializeUiLanguage: async () => languageSettings,
    updateUiLanguage: async () => languageSettings,
  };
  return render(
    <LanguageProvider api={languageApi} detectedLanguages={['zh-CN']}>
      <MemoryRouter initialEntries={['/library']}>
        <Routes>
          <Route
            path="/library"
            element={
              <LibraryPage libraryApi={api} importCoordinator={coordinator} />
            }
          />
          <Route path="/books/:bookId/read" element={<h1>阅读</h1>} />
        </Routes>
        <LocationProbe />
      </MemoryRouter>
    </LanguageProvider>,
  );
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('library import flow', () => {
  it('uses the exact picker filter and picker cancellation does nothing', async () => {
    const user = userEvent.setup();
    const api = new FakeLibraryApi();
    const coordinator = new FakeCoordinator();
    openMock.mockResolvedValue(null);
    renderFlow(api, coordinator);

    await user.click(await screen.findByRole('button', { name: '导入教材' }));

    expect(openMock).toHaveBeenCalledWith({
      multiple: false,
      directory: false,
      filters: [{ name: '教材', extensions: ['pdf', 'epub', 'docx'] }],
    });
    expect(coordinator.importDocument).not.toHaveBeenCalled();
    expect(screen.getByTestId('location')).toHaveTextContent('/library');
  });

  it('shows ordered stages and keeps cancellation available before and after Rust returns the id', async () => {
    const user = userEvent.setup();
    const api = new FakeLibraryApi();
    const coordinator = new FakeCoordinator();
    openMock.mockResolvedValue('C:\\private\\fixture.pdf');
    let progress!: (event: ImportEvent) => void;
    let identify!: (book: BookSummary) => void;
    let rejectImport!: (reason: UserFacingError) => void;
    coordinator.importImpl = async (_path, onProgress, onBook) => {
      progress = onProgress;
      identify = onBook;
      return await new Promise<BookSummary>((_resolve, reject) => {
        rejectImport = reject;
      });
    };
    renderFlow(api, coordinator);
    await user.click(await screen.findByRole('button', { name: '导入教材' }));

    act(() =>
      progress({
        stage: 'copying',
        completed: 1,
        total: 4,
        messageKey: 'copy',
      }),
    );
    const stages = screen
      .getAllByRole('listitem')
      .map((item) => item.textContent);
    expect(stages).toEqual(['复制文件', '解析内容', '建立本地索引']);
    expect(screen.getByText('复制文件')).toHaveAttribute(
      'aria-current',
      'step',
    );
    await user.click(screen.getByRole('button', { name: '取消导入' }));
    expect(coordinator.cancelPending).toHaveBeenCalledOnce();

    act(() => {
      identify(book({ importStatus: 'parsing' }));
      progress({
        stage: 'parsing',
        completed: 2,
        total: 4,
        messageKey: 'parse',
      });
    });
    expect(screen.getByText('解析内容')).toHaveAttribute(
      'aria-current',
      'step',
    );
    expect(screen.getByRole('button', { name: '取消导入' })).toBeEnabled();
    await user.click(screen.getByRole('button', { name: '取消导入' }));
    expect(coordinator.cancel).toHaveBeenCalledWith(BOOK_ID);

    act(() =>
      progress({
        stage: 'indexing',
        completed: 3,
        total: 4,
        messageKey: 'index',
      }),
    );
    expect(screen.getByText('建立本地索引')).toHaveAttribute(
      'aria-current',
      'step',
    );
    expect(screen.getByRole('button', { name: '取消导入' })).toBeEnabled();
    act(() =>
      rejectImport({
        code: 'IMPORT_CANCELLED',
        message: '导入已取消。',
        nextStep: '重新开始导入。',
        diagnosticId: null,
      }),
    );
    await waitFor(() =>
      expect(
        screen.queryByRole('progressbar', { name: '导入进度' }),
      ).not.toBeInTheDocument(),
    );
    expect(document.body).not.toHaveTextContent('C:\\private\\fixture.pdf');
  });

  it.each([
    ['successful import', BOOK_ID],
    ['duplicate import', EXISTING_ID],
  ])('opens the Rust-returned book for a %s', async (_label, returnedId) => {
    const user = userEvent.setup();
    const api = new FakeLibraryApi();
    const coordinator = new FakeCoordinator();
    openMock.mockResolvedValue('C:\\transient\\fixture.pdf');
    coordinator.importImpl = async () => book({ id: returnedId });
    renderFlow(api, coordinator);

    await user.click(await screen.findByRole('button', { name: '导入教材' }));

    await waitFor(() =>
      expect(screen.getByTestId('location')).toHaveTextContent(
        `/books/${returnedId}/read`,
      ),
    );
    expect(document.body).not.toHaveTextContent('C:\\transient\\fixture.pdf');
  });

  it('shows a safe failure and supports reselect retry', async () => {
    const user = userEvent.setup();
    const failed = book({
      importStatus: 'failed',
      importErrorCode: 'FILE_CORRUPTED',
      importErrorMessage: '文件已损坏或无法读取。',
      importErrorStage: 'parsing',
    });
    const api = new FakeLibraryApi();
    api.books = [failed];
    const coordinator = new FakeCoordinator();
    renderFlow(api, coordinator);

    expect(await screen.findByText('文件已损坏或无法读取。')).toBeVisible();
    openMock.mockResolvedValueOnce('C:\\transient\\replacement.pdf');
    coordinator.retryImpl = async () => book();
    await user.click(screen.getByRole('button', { name: '重新选择并重试' }));
    await waitFor(() => expect(coordinator.retryDocument).toHaveBeenCalled());
    expect(coordinator.retryDocument).toHaveBeenCalledWith(
      BOOK_ID,
      'C:\\transient\\replacement.pdf',
      expect.any(Function),
      expect.any(Function),
    );
    expect(document.body).not.toHaveTextContent(
      'C:\\transient\\replacement.pdf',
    );
  });

  it('deletes a failed import record', async () => {
    const user = userEvent.setup();
    const api = new FakeLibraryApi();
    api.books = [
      book({
        importStatus: 'failed',
        importErrorCode: 'FILE_CORRUPTED',
        importErrorMessage: '文件已损坏或无法读取。',
        importErrorStage: 'parsing',
      }),
    ];
    const coordinator = new FakeCoordinator();
    renderFlow(api, coordinator);

    await user.click(
      await screen.findByRole('button', { name: '删除失败记录' }),
    );
    expect(api.deleteFailedImport).toHaveBeenCalledWith(BOOK_ID);
    await waitFor(() =>
      expect(screen.queryByText('fixture.pdf')).not.toBeInTheDocument(),
    );
  });
});
