import type { ReactNode } from 'react';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { LanguageContext } from '../../app/LanguageProvider';
import { formatMessage } from '../../lib/i18n';
import { IndexStartPage, type IndexStartPageProps } from './IndexStartPage';

const BOOK_ID = '11111111-1111-4111-8111-111111111111';
const PROFILE_ID = '22222222-2222-4222-8222-222222222222';
const RUN_ID = '44444444-4444-4444-8444-444444444444';

afterEach(cleanup);

describe('IndexStartPage', () => {
  it('rejects after local inspection without starting anything and keeps only abnormal pages', async () => {
    const source = new Uint8Array([1, 2, 3]);
    let inspectedSource: ArrayBuffer | undefined;
    const dependencies = createDependencies({ source });
    dependencies.inspectQuality.mockImplementation(async (localSource) => {
      inspectedSource = localSource;
      return [
        quality(1, 'reliable_text', 'not_required'),
        quality(2, 'no_text', 'needs_review'),
      ];
    });
    renderPage(dependencies);

    expect(
      await screen.findByRole('dialog', {
        name: 'Start AI-assisted indexing?',
      }),
    ).toBeVisible();
    expect(
      screen.getByText(
        /Profile: Verified vision\. Model: vision-model\. Pages: 1/,
      ),
    ).toBeVisible();
    expect([...source]).toEqual([0, 0, 0]);
    expect(inspectedSource).toBeDefined();
    if (!inspectedSource) throw new Error('quality inspector did not run');
    expect([...new Uint8Array(inspectedSource)]).toEqual([0, 0, 0]);

    await userEvent.click(
      screen.getByRole('button', { name: 'Continue without AI indexing' }),
    );

    expect(
      screen.getByText(`location:/books/${BOOK_ID}/read?index=local-only`),
    ).toBeVisible();
    expect(dependencies.confirmOperation).not.toHaveBeenCalled();
    expect(dependencies.createRun).not.toHaveBeenCalled();
    expect(dependencies.updateConsent).not.toHaveBeenCalled();
  });

  it('keeps confirmation and rejection reachable after PDF.js transfers the inspection buffer', async () => {
    const source = new Uint8Array([7, 8, 9]);
    let transferredSource: ArrayBuffer | undefined;
    const dependencies = createDependencies({ source });
    dependencies.inspectQuality.mockImplementation(async (localSource) => {
      transferredSource = structuredClone(localSource, {
        transfer: [localSource],
      });
      expect(localSource.byteLength).toBe(0);
      return [quality(1, 'no_text', 'needs_review')];
    });
    renderPage(dependencies);

    expect(
      await screen.findByRole('dialog', {
        name: 'Start AI-assisted indexing?',
      }),
    ).toBeVisible();
    expect([...source]).toEqual([0, 0, 0]);
    expect(transferredSource).toBeDefined();
    expect(transferredSource?.byteLength).toBe(3);

    await userEvent.click(
      screen.getByRole('button', { name: 'Continue without AI indexing' }),
    );

    expect(
      screen.getByText(`location:/books/${BOOK_ID}/read?index=local-only`),
    ).toBeVisible();
    expect(dependencies.confirmOperation).not.toHaveBeenCalled();
    expect(dependencies.createRun).not.toHaveBeenCalled();
    expect(dependencies.updateConsent).not.toHaveBeenCalled();
    if (transferredSource) new Uint8Array(transferredSource).fill(0);
  });

  it('uses the exact verified default profile and creates one run from one fresh token', async () => {
    const dependencies = createDependencies({
      source: new Uint8Array([1, 2, 3]),
    });
    renderPage(dependencies);

    await userEvent.click(
      await screen.findByRole('checkbox', {
        name: 'Do not show this index-start prompt again for this profile',
      }),
    );
    await userEvent.click(
      screen.getByRole('button', { name: 'Confirm and start' }),
    );

    await waitFor(() =>
      expect(screen.getByText(`location:/quality/${RUN_ID}`)).toBeVisible(),
    );
    expect(dependencies.listProfiles).toHaveBeenCalledWith(
      'structured_page_analysis',
    );
    expect(dependencies.confirmOperation).toHaveBeenCalledTimes(1);
    const request = dependencies.confirmOperation.mock.calls[0]?.[0];
    expect(request).toMatchObject({
      runId: null,
      bookId: BOOK_ID,
      sourceSha256:
        '039058c6f2c0cb492c533b0a4d14ef77cc0f78abccced5287d84a1a2011cfb81',
      providerProfileId: PROFILE_ID,
      pages: [
        {
          pageNumber: 2,
          qualityReason: 'no_text',
          localTextSha256: null,
        },
      ],
    });
    expect(dependencies.updateConsent).toHaveBeenCalledWith(
      PROFILE_ID,
      'ai_index',
      'skip_prompt',
    );
    expect(dependencies.createRun).toHaveBeenCalledTimes(1);
    expect(dependencies.createRun).toHaveBeenCalledWith(
      '33333333-3333-4333-8333-333333333333',
      request,
    );
  });

  it('fails safely when no exact verified vision default is available', async () => {
    const dependencies = createDependencies({
      source: new Uint8Array([1, 2, 3]),
    });
    dependencies.listProfiles.mockResolvedValue([
      {
        ...profile(),
        id: '55555555-5555-4555-8555-555555555555',
      },
    ]);
    renderPage(dependencies);

    expect(
      await screen.findByText(/Choose a verified vision profile/),
    ).toBeVisible();
    expect(dependencies.confirmOperation).not.toHaveBeenCalled();
    expect(dependencies.createRun).not.toHaveBeenCalled();
    await userEvent.click(
      screen.getByRole('button', { name: 'Configure AI services' }),
    );
    expect(screen.getByText('location:/ai-services')).toBeVisible();
  });

  it('does not resolve a provider for non-PDF or reliable-only books', async () => {
    const nonPdf = createDependencies({
      source: new Uint8Array([1, 2, 3]),
      format: 'epub',
    });
    const first = renderPage(nonPdf);
    expect(
      await screen.findByText(/available only for ready PDF books/),
    ).toBeVisible();
    expect(nonPdf.readBookSource).not.toHaveBeenCalled();
    expect(nonPdf.listProfiles).not.toHaveBeenCalled();
    first.unmount();

    const reliable = createDependencies({
      source: new Uint8Array([1, 2, 3]),
    });
    reliable.inspectQuality.mockResolvedValue([
      quality(1, 'reliable_text', 'not_required'),
    ]);
    renderPage(reliable);
    expect(
      await screen.findByText(/already has reliable local text/),
    ).toBeVisible();
    expect(reliable.listProfiles).not.toHaveBeenCalled();
    expect(reliable.confirmOperation).not.toHaveBeenCalled();
  });

  it('keeps local quality failures actionable and sends nothing', async () => {
    const source = new Uint8Array([4, 5, 6]);
    const dependencies = createDependencies({ source });
    dependencies.inspectQuality.mockRejectedValue(
      new Error('synthetic local inspection failure'),
    );
    renderPage(dependencies);

    expect(
      await screen.findByText(/could not be checked locally/),
    ).toBeVisible();
    expect([...source]).toEqual([0, 0, 0]);
    expect(dependencies.getSettings).not.toHaveBeenCalled();
    expect(dependencies.listProfiles).not.toHaveBeenCalled();
    expect(dependencies.confirmOperation).not.toHaveBeenCalled();
    expect(dependencies.createRun).not.toHaveBeenCalled();
  });

  it('wipes a source that arrives after unmount without inspecting or starting', async () => {
    let releaseSource: (source: Uint8Array) => void = () => {};
    const source = new Uint8Array([9, 8, 7]);
    const dependencies = createDependencies({ source });
    dependencies.readBookSource.mockImplementation(
      () =>
        new Promise<Uint8Array>((resolve) => {
          releaseSource = resolve;
        }),
    );
    const view = renderPage(dependencies);
    await waitFor(() =>
      expect(dependencies.readBookSource).toHaveBeenCalledOnce(),
    );

    view.unmount();
    releaseSource(source);

    await waitFor(() => expect([...source]).toEqual([0, 0, 0]));
    expect(dependencies.inspectQuality).not.toHaveBeenCalled();
    expect(dependencies.listProfiles).not.toHaveBeenCalled();
    expect(dependencies.confirmOperation).not.toHaveBeenCalled();
  });
});

function createDependencies({
  source,
  format = 'pdf',
}: {
  source: Uint8Array;
  format?: 'pdf' | 'epub' | 'docx';
}) {
  return {
    getReaderBootstrap: vi.fn().mockResolvedValue({
      book: {
        id: BOOK_ID,
        title: 'Synthetic fixture',
        originalFilename: 'synthetic.pdf',
        author: null,
        language: null,
        format,
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
        createdAt: '2026-08-04T00:00:00Z',
        updatedAt: '2026-08-04T00:00:00Z',
        lastOpenedAt: null,
      },
      lastLocator: null,
    }),
    readBookSource: vi.fn().mockResolvedValue(source),
    getSettings: vi.fn().mockResolvedValue({
      onboardingCompleted: true,
      activeProviderProfileId: PROFILE_ID,
      defaultLearningProfileId: PROFILE_ID,
      defaultVisionProfileId: PROFILE_ID,
      theme: 'system',
      contextMode: 'standard',
      uiLanguage: 'en',
      uiLanguageInitialized: true,
      firstReaderHintCompleted: true,
    }),
    listProfiles: vi.fn().mockResolvedValue([profile()]),
    updateConsent: vi.fn().mockResolvedValue(undefined),
    confirmOperation: vi
      .fn()
      .mockResolvedValue('33333333-3333-4333-8333-333333333333'),
    createRun: vi.fn().mockResolvedValue(RUN_ID),
    inspectQuality: vi
      .fn()
      .mockResolvedValue([quality(2, 'no_text', 'needs_review')]),
  };
}

function profile() {
  return {
    id: PROFILE_ID,
    kind: 'openai' as const,
    displayName: 'Verified vision',
    modelId: 'vision-model',
    contextWindowTokens: 128_000,
    isActive: true,
    credentialStatus: 'available' as const,
    validatedAt: '2026-08-04T00:00:00Z',
  };
}

function quality(
  pageNumber: number,
  qualityReason: 'reliable_text' | 'no_text',
  status: 'not_required' | 'needs_review',
) {
  return { schemaVersion: 1 as const, pageNumber, qualityReason, status };
}

function renderPage(dependencies: ReturnType<typeof createDependencies>) {
  const props: IndexStartPageProps = {
    readerApi: {
      getReaderBootstrap: dependencies.getReaderBootstrap,
      readBookSource: dependencies.readBookSource,
    },
    providerApi: {
      getSettings: dependencies.getSettings,
      listProfiles: dependencies.listProfiles,
      updateConsent: dependencies.updateConsent,
    },
    indexingApi: {
      confirmOperation: dependencies.confirmOperation,
      createRun: dependencies.createRun,
    },
    inspectQuality: dependencies.inspectQuality,
  };
  return render(
    <LanguageHarness>
      <MemoryRouter initialEntries={[`/books/${BOOK_ID}/index-start`]}>
        <Routes>
          <Route
            path="/books/:bookId/index-start"
            element={<IndexStartPage {...props} />}
          />
          <Route path="/books/:bookId/read" element={<Location />} />
          <Route
            path="/books/:bookId/index-quality/:runId"
            element={<QualityLocation />}
          />
          <Route path="/ai-services" element={<Location />} />
        </Routes>
      </MemoryRouter>
    </LanguageHarness>,
  );
}

function Location() {
  const location = useLocation();
  return <p>{`location:${location.pathname}${location.search}`}</p>;
}

function QualityLocation() {
  const location = useLocation();
  return <p>{`location:/quality/${location.pathname.split('/').at(-1)}`}</p>;
}

function LanguageHarness({ children }: { children: ReactNode }) {
  return (
    <LanguageContext.Provider
      value={{
        uiLanguage: 'en',
        isLoading: false,
        statusMessage: null,
        switchLanguage: async () => {},
        message: (key, values) => formatMessage('en', key, values),
      }}
    >
      {children}
    </LanguageContext.Provider>
  );
}
