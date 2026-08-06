import { afterEach, describe, expect, it } from 'vitest';

import { clearMocks, installTauriMock } from '../../test/tauri-mock';
import { TauriLearningOverviewApi } from './api';

const BOOK_ID = '11111111-1111-4111-8111-111111111111';
const SECTION_ID = '22222222-2222-4222-8222-222222222222';
const PRIVATE_SENTINEL = 'PRIVATE_OVERVIEW_CONTENT_SENTINEL';

afterEach(() => clearMocks());

describe('TauriLearningOverviewApi', () => {
  it('accepts, cross-checks, and deeply freezes a content-free overview', async () => {
    const calls = installTauriMock((command) => {
      expect(command).toBe('get_learning_overview');
      return overview();
    });
    const result = await new TauriLearningOverviewApi().get(BOOK_ID);

    expect(result.sources.map((source) => source.source)).toEqual([
      'local_text',
      'ai_transcribed',
      'ai_description',
      'user_corrected',
      'user_note',
      'history_summary',
    ]);
    expect(Object.isFrozen(result)).toBe(true);
    expect(Object.isFrozen(result.sections[0])).toBe(true);
    expect(Object.isFrozen(result.sources)).toBe(true);
    expect(calls).toHaveLength(1);
    expect(JSON.stringify(calls[0]?.payload)).not.toContain(PRIVATE_SENTINEL);
  });

  it('rejects malformed, extra, reordered, and inconsistent source data', async () => {
    const malformed = [
      { ...overview(), noteBody: PRIVATE_SENTINEL },
      {
        ...overview(),
        sources: [...overview().sources].reverse(),
      },
      {
        ...overview(),
        sources: overview().sources.map((source) =>
          source.source === 'ai_description'
            ? { ...source, quoteableAsTextbook: true }
            : source,
        ),
      },
      {
        ...overview(),
        activity: { ...overview().activity, userNoteCount: 2 },
      },
    ];
    const api = new TauriLearningOverviewApi();
    for (const response of malformed) {
      installTauriMock(() => response);
      await expect(api.get(BOOK_ID)).rejects.toEqual({
        code: 'LEARNING_OVERVIEW_DATA_INVALID',
      });
    }
  });

  it('keeps invocation failures code-only and rejects invalid IDs before IPC', async () => {
    installTauriMock(() => {
      throw {
        code: 'LEARNING_OVERVIEW_BUSY',
        message: PRIVATE_SENTINEL,
        diagnosticId: PRIVATE_SENTINEL,
      };
    });
    const api = new TauriLearningOverviewApi();
    await expect(api.get(BOOK_ID)).rejects.toEqual({
      code: 'LEARNING_OVERVIEW_BUSY',
    });

    const calls = installTauriMock(() => overview());
    await expect(api.get('not-a-book-id')).rejects.toEqual({
      code: 'LEARNING_OVERVIEW_INVALID_INPUT',
    });
    expect(calls).toHaveLength(0);
  });
});

function overview() {
  return {
    bookId: BOOK_ID,
    format: 'pdf',
    teachingInstructionConfigured: true,
    sectionCount: 1,
    sections: [
      {
        id: SECTION_ID,
        parentId: null,
        ordinal: 0,
        title: 'Visible synthetic section',
        localTextItemCount: 2,
        userNoteCount: 1,
        completedConversationCount: 1,
        completedExchangeCount: 1,
      },
    ],
    sources: [
      source('local_text', 2, 1, 0, true),
      source('ai_transcribed', 1, 0, 1, true),
      source('ai_description', 1, 0, 1, false),
      source('user_corrected', 1, 0, 1, true),
      source('user_note', 1, 1, 0, false),
      source('history_summary', 1, 1, 0, false),
    ],
    activity: {
      userNoteCount: 1,
      completedConversationCount: 1,
      completedExchangeCount: 1,
      citationCount: 1,
    },
  };
}

function source(
  name: string,
  itemCount: number,
  coveredSectionCount: number,
  coveredPageCount: number,
  quoteableAsTextbook: boolean,
) {
  return {
    source: name,
    itemCount,
    coveredSectionCount,
    coveredPageCount,
    quoteableAsTextbook,
  };
}
