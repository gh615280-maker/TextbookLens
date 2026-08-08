import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import { LanguageContext } from '../../app/LanguageProvider';
import { formatMessage } from '../../lib/i18n';
import { IndexingCoordinator } from './IndexingCoordinator';
import { IndexingProvider } from './IndexingProvider';
import { IndexQualityPage } from './IndexQualityPage';
import type { IndexingApi } from './api';

const RUN_ID = '11111111-1111-4111-8111-111111111111';
const BOOK_ID = '22222222-2222-4222-822222222222';

function api(): IndexingApi {
  return {
    confirmOperation: vi.fn(),
    createRun: vi.fn(),
    authorizeRun: vi.fn(),
    claimRenderBatch: vi.fn().mockResolvedValue({ claims: [] }),
    readClaimedSource: vi.fn(),
    submitRenderedBatch: vi.fn(),
    reportRenderFailure: vi.fn(),
    pauseRun: vi.fn(),
    resumeRun: vi.fn(),
    cancelRun: vi.fn(),
    retryPage: vi.fn(),
    getPageReview: vi.fn(),
    listPageCorrections: vi.fn(),
    saveCorrection: vi.fn(),
    resolveCorrectionConflict: vi.fn(),
    deleteCorrection: vi.fn(),
    getRunAggregate: vi.fn().mockResolvedValue({
      runId: RUN_ID,
      bookId: BOOK_ID,
      controlStatus: 'running',
      aggregateStatus: 'partial',
      pages: {
        total: 2,
        notRequired: 0,
        queued: 1,
        rendering: 0,
        sending: 0,
        parsing: 0,
        validating: 0,
        indexed: 0,
        needsReview: 1,
        failed: 0,
        cancelled: 0,
      },
      updatedAt: '2026-08-04T00:00:00.000Z',
    }),
    listPageReviews: vi.fn().mockResolvedValue([
      {
        id: '33333333-3333-4333-833333333333',
        runId: RUN_ID,
        bookId: BOOK_ID,
        pageNumber: 2,
        qualityReason: 'no_text',
        status: 'needs_review',
        reviewReason: null,
        safeError: null,
        contentVersion: 1,
        blocks: [],
        corrections: [],
        updatedAt: '2026-08-04T00:00:00.000Z',
      },
    ]),
  };
}

describe('IndexQualityPage', () => {
  it('loads durable state and controls the persisted run without provider calls', async () => {
    const indexingApi = api();
    const coordinator = new IndexingCoordinator(indexingApi, vi.fn());
    const user = userEvent.setup();
    render(
      <LanguageContext.Provider
        value={{
          uiLanguage: 'en',
          isLoading: false,
          statusMessage: null,
          switchLanguage: async () => {},
          message: (key, values) => formatMessage('en', key, values),
        }}
      >
        <IndexingProvider coordinator={coordinator}>
          <MemoryRouter
            initialEntries={[`/books/${BOOK_ID}/index-quality/${RUN_ID}`]}
          >
            <Routes>
              <Route
                path="/books/:bookId/index-quality/:runId"
                element={<IndexQualityPage api={indexingApi} />}
              />
            </Routes>
          </MemoryRouter>
        </IndexingProvider>
      </LanguageContext.Provider>,
    );
    expect(await screen.findByText('Index quality')).toBeVisible();
    const controls = screen.getByRole('group', {
      name: 'Index run controls',
    });
    expect(
      within(controls).getByRole('button', { name: 'Pause run' }),
    ).toBeVisible();
    expect(
      within(controls).getByRole('button', { name: 'Cancel run' }),
    ).toBeVisible();
    await user.click(
      within(controls).getByRole('button', { name: 'Pause run' }),
    );
    await waitFor(() =>
      expect(indexingApi.pauseRun).toHaveBeenCalledWith(RUN_ID),
    );
  });
});
