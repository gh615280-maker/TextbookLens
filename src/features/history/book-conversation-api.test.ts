import { afterEach, describe, expect, it } from 'vitest';

import { clearMocks, installTauriMock } from '../../test/tauri-mock';
import {
  TauriBookConversationApi,
  type BookConversationHistory,
} from './conversation-api';

const BOOK = '11111111-1111-4111-8111-111111111111';
const CONVERSATION = '22222222-2222-4222-8222-222222222222';
const PROVIDER = '33333333-3333-4333-8333-333333333333';

afterEach(() => clearMocks());

describe('TauriBookConversationApi', () => {
  it('loads and deeply freezes only a strict book-scope history', async () => {
    const calls = installTauriMock((command) => {
      expect(command).toBe('get_book_learning_conversation');
      return history();
    });
    const api = new TauriBookConversationApi();
    const loaded = await api.getBook(BOOK, CONVERSATION);
    expect(loaded.scope).toBe('book');
    expect(loaded.messages).toHaveLength(4);
    expect(loaded.messages[1]).toMatchObject({
      providerId: PROVIDER,
      modelId: 'captured-model-v1',
    });
    expect(loaded.messages[3].modelId).toBe('captured-model-v2');
    expect(Object.isFrozen(loaded)).toBe(true);
    expect(Object.isFrozen(loaded.messages)).toBe(true);
    expect(Object.isFrozen(loaded.messages[1])).toBe(true);
    expect(calls).toEqual([
      {
        command: 'get_book_learning_conversation',
        payload: { bookId: BOOK, conversationId: CONVERSATION },
      },
    ]);
  });

  it('rejects annotation fields, wrong actions, cross-book citations and odd history', async () => {
    const api = new TauriBookConversationApi();
    for (const invalid of [
      { ...history(), annotationId: '44444444-4444-4444-8444-444444444444' },
      {
        ...history(),
        messages: history().messages.map((message, index) =>
          index === 0 ? { ...message, action: 'overview' } : message,
        ),
      },
      { ...history(), messages: history().messages.slice(0, 3) },
      {
        ...history(),
        messages: history().messages.map((message, index) =>
          index === 1
            ? {
                ...message,
                citations: [
                  {
                    id: 'TL-C1',
                    label: 'Page 1',
                    bookId: '55555555-5555-4555-8555-555555555555',
                    sectionId: null,
                    locator: {
                      format: 'pdf',
                      startPage: 1,
                      endPage: 1,
                      rectsByPage: null,
                    },
                    source: 'local_text',
                    reviewStatus: 'not_required',
                    quoteable: true,
                  },
                ],
              }
            : message,
        ),
      },
    ]) {
      installTauriMock(() => invalid);
      await expect(api.getBook(BOOK, CONVERSATION)).rejects.toBeDefined();
    }
  });

  it('deletes by book and conversation IDs without an annotation or content payload', async () => {
    const calls = installTauriMock((command, payload) => {
      expect(command).toBe('delete_book_learning_conversation');
      expect(payload).not.toHaveProperty('annotationId');
      expect(payload).not.toHaveProperty('messages');
      return undefined;
    });
    const api = new TauriBookConversationApi();
    await api.deleteBook(BOOK, CONVERSATION);
    expect(calls).toEqual([
      {
        command: 'delete_book_learning_conversation',
        payload: { bookId: BOOK, conversationId: CONVERSATION },
      },
    ]);
  });
});

function history(): BookConversationHistory {
  return {
    id: CONVERSATION,
    bookId: BOOK,
    scope: 'book',
    status: 'completed',
    messages: [
      message(0, 'user', 'ask', 'Initial book question.', null, null),
      message(
        1,
        'assistant',
        'ask',
        'Initial answer.',
        PROVIDER,
        'captured-model-v1',
      ),
      message(2, 'user', 'continue', 'Continue?', null, null),
      message(
        3,
        'assistant',
        'continue',
        'Current answer.',
        PROVIDER,
        'captured-model-v2',
      ),
    ],
    createdAt: '2026-08-06T00:00:00.000Z',
    updatedAt: '2026-08-06T00:01:00.000Z',
  };
}

function message(
  ordinal: number,
  role: 'user' | 'assistant',
  action: 'ask' | 'continue',
  content: string,
  providerId: string | null,
  modelId: string | null,
) {
  return {
    id: `${ordinal + 6}6666666-6666-4666-8666-666666666666`.slice(0, 36),
    ordinal,
    role,
    action,
    content,
    providerId,
    modelId,
    citations: [],
    createdAt:
      ordinal < 2 ? '2026-08-06T00:00:00.000Z' : '2026-08-06T00:01:00.000Z',
  } as const;
}
