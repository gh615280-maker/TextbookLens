import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import { LanguageProvider } from '../../app/LanguageProvider';
import type { LearningOverview } from '../../lib/generated/overview';
import { OverviewPage } from './OverviewPage';

const BOOK = '11111111-1111-4111-8111-111111111111';

describe('OverviewPage', () => {
  it('keeps the local overview and completed history usable when AI is unavailable', async () => {
    const overviewApi = { get: vi.fn(async () => overview()) };
    const historyApi = {
      listBook: vi.fn(async () => [
        {
          id: '33333333-3333-4333-8333-333333333333',
          createdAt: '2026-08-06T00:00:00.000Z',
          updatedAt: '2026-08-06T00:01:00.000Z',
          messageCount: 2,
          firstQuestionPreview: 'Synthetic safe question',
        },
      ]),
      getBook: vi.fn(),
      deleteBook: vi.fn(),
    };
    const preparationApi = {
      prepare: vi.fn(),
      authorize: vi.fn(),
      discard: vi.fn(),
      start: vi.fn(),
    };
    const { container } = render(
      <LanguageProvider
        api={{
          getAppSettings: async () => ({
            uiLanguage: 'en',
            uiLanguageInitialized: true,
            onboardingCompleted: true,
            activeProviderProfileId: null,
            theme: 'system',
            contextMode: 'standard',
            firstReaderHintCompleted: false,
          }),
          initializeUiLanguage: vi.fn(),
          updateUiLanguage: vi.fn(),
        }}
      >
        <MemoryRouter initialEntries={[`/books/${BOOK}/overview`]}>
          <Routes>
            <Route
              path="/books/:bookId/overview"
              element={
                <OverviewPage
                  overviewApi={overviewApi}
                  historyApi={historyApi}
                  preparationApi={preparationApi}
                />
              }
            />
          </Routes>
        </MemoryRouter>
      </LanguageProvider>,
    );
    await screen.findByText('Synthetic safe question');
    const questionHeading = screen.getByRole('heading', {
      name: 'Ask about this textbook',
    });
    const collapsibles = Array.from(container.querySelectorAll('details'));
    expect(collapsibles).toHaveLength(2);
    expect(collapsibles.every((details) => !details.open)).toBe(true);
    expect(questionHeading.compareDocumentPosition(collapsibles[0]!)).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING,
    );

    await userEvent.click(screen.getByText('Local learning overview'));
    expect(collapsibles[0]).toHaveAttribute('open');

    expect(screen.getByText('local_text')).toBeInTheDocument();
    expect(screen.getByText('ai_transcribed')).toBeInTheDocument();
    expect(screen.getByText('ai_description')).toBeInTheDocument();
    expect(screen.getByText('user_corrected')).toBeInTheDocument();
    expect(screen.getByText('user_note')).toBeInTheDocument();
    expect(screen.getByText('history_summary')).toBeInTheDocument();
    expect(overviewApi.get).toHaveBeenCalledWith(BOOK);
    expect(historyApi.listBook).toHaveBeenCalledWith(BOOK);
    expect(preparationApi.prepare).not.toHaveBeenCalled();
  });
});

function overview(): LearningOverview {
  return {
    bookId: BOOK,
    format: 'pdf',
    teachingInstructionConfigured: false,
    sectionCount: 1,
    sections: [
      {
        id: '22222222-2222-4222-8222-222222222222',
        parentId: null,
        ordinal: 0,
        title: 'Synthetic section',
        localTextItemCount: 1,
        userNoteCount: 0,
        completedConversationCount: 1,
        completedExchangeCount: 1,
      },
    ],
    sources: [
      source('local_text', true),
      source('ai_transcribed', true),
      source('ai_description', false),
      source('user_corrected', true),
      source('user_note', false),
      source('history_summary', false),
    ],
    activity: {
      userNoteCount: 0,
      completedConversationCount: 1,
      completedExchangeCount: 1,
      citationCount: 0,
    },
  };
}

function source(
  source: LearningOverview['sources'][number]['source'],
  quoteableAsTextbook: boolean,
) {
  return {
    source,
    itemCount: source === 'local_text' || source === 'history_summary' ? 1 : 0,
    coveredSectionCount:
      source === 'local_text' || source === 'history_summary' ? 1 : 0,
    coveredPageCount: 0,
    quoteableAsTextbook,
  };
}
