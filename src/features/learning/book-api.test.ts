import { afterEach, describe, expect, it } from 'vitest';

import { clearMocks, installTauriMock } from '../../test/tauri-mock';
import type { PrepareBookLearningRequestMetadata } from '../../lib/generated/book_learning';
import { TauriBookLearningPreparationApi } from './book-api';

const BOOK_ID = '11111111-1111-4111-8111-111111111111';
const CONVERSATION_ID = '22222222-2222-4222-8222-222222222222';
const PREPARATION_ID = '33333333-3333-4333-8333-333333333333';
const QUESTION = 'How does the synthetic lattice behave?';
const PRIVATE_SENTINEL = 'PRIVATE_BOOK_CONTEXT_SENTINEL';

afterEach(() => clearMocks());

describe('TauriBookLearningPreparationApi', () => {
  it('prepares new and continued questions with routing-only IPC metadata', async () => {
    const calls = installTauriMock((command, payload) => {
      expect(command).toBe('prepare_book_learning_request');
      const metadata = (payload as { metadata?: unknown }).metadata;
      expect(metadata).not.toHaveProperty('toc');
      expect(metadata).not.toHaveProperty('retrievalSnippets');
      expect(metadata).not.toHaveProperty('history');
      expect(metadata).not.toHaveProperty('teachingInstruction');
      expect(metadata).not.toHaveProperty('providerProfileId');
      expect(metadata).not.toHaveProperty('modelId');
      return safeSummary();
    });
    const api = new TauriBookLearningPreparationApi();

    const first = await api.prepare({
      kind: 'new',
      bookId: BOOK_ID,
      question: QUESTION,
    });
    expect(first).toEqual(safeSummary());
    expect(Object.isFrozen(first)).toBe(true);
    expect(Object.isFrozen(first.riskFlags)).toBe(true);

    await api.prepare({
      kind: 'continue',
      bookId: BOOK_ID,
      conversationId: CONVERSATION_ID,
      question: '继续分析这个例子。',
    });
    expect(calls).toEqual([
      {
        command: 'prepare_book_learning_request',
        payload: {
          metadata: {
            kind: 'new',
            hasQuestion: true,
            hasConversation: false,
          },
        },
      },
      {
        command: 'prepare_book_learning_request',
        payload: {
          metadata: {
            kind: 'continue',
            hasQuestion: true,
            hasConversation: true,
          },
        },
      },
    ]);
    expect(JSON.stringify(calls)).not.toContain(QUESTION);
    expect(JSON.stringify(calls)).not.toContain(BOOK_ID);
    expect(JSON.stringify(calls)).not.toContain(CONVERSATION_ID);
  });

  it('rejects frontend-supplied authority fields and malformed Unicode before IPC', async () => {
    const calls = installTauriMock(() => safeSummary());
    const api = new TauriBookLearningPreparationApi();
    for (const forbidden of [
      'toc',
      'retrievalSnippets',
      'history',
      'teachingInstruction',
      'providerProfileId',
      'modelId',
      'profileSnapshot',
      'prompt',
      'citations',
      'bookContent',
    ]) {
      const metadata = {
        kind: 'new',
        bookId: BOOK_ID,
        question: QUESTION,
        [forbidden]: PRIVATE_SENTINEL,
      } as PrepareBookLearningRequestMetadata;
      await expect(api.prepare(metadata)).rejects.toEqual({
        code: 'INVALID_INPUT',
      });
    }
    for (const question of [
      '',
      ' \n\t ',
      'control\u0001character',
      'carriage\rreturn',
      '\ud800',
      '界'.repeat(16_385),
      '🧭'.repeat(16_385),
    ]) {
      await expect(
        api.prepare({ kind: 'new', bookId: BOOK_ID, question }),
      ).rejects.toEqual({ code: 'INVALID_INPUT' });
    }
    expect(calls).toHaveLength(0);
  });

  it('accepts only a content-free internally consistent summary', async () => {
    const api = new TauriBookLearningPreparationApi();
    installTauriMock(() => ({
      ...safeSummary(),
      question: PRIVATE_SENTINEL,
      locator: PRIVATE_SENTINEL,
      providerProfileId: BOOK_ID,
    }));
    await expect(
      api.prepare({ kind: 'new', bookId: BOOK_ID, question: QUESTION }),
    ).rejects.toEqual({ code: 'DATABASE_ERROR' });

    installTauriMock(() => ({
      ...safeSummary(),
      estimatedInputTokens: 32_001,
      riskFlags: [],
    }));
    await expect(
      api.prepare({ kind: 'new', bookId: BOOK_ID, question: QUESTION }),
    ).rejects.toEqual({ code: 'DATABASE_ERROR' });
  });

  it('authorizes and discards without exposing IDs in mock records', async () => {
    const calls = installTauriMock(() => undefined);
    const api = new TauriBookLearningPreparationApi();
    await api.authorize(PREPARATION_ID, 'allow');
    await api.discard(PREPARATION_ID);
    expect(calls).toEqual([
      {
        command: 'authorize_book_learning_request',
        payload: { decision: 'allow' },
      },
      {
        command: 'discard_book_learning_preparation',
        payload: { hasPreparationId: true },
      },
    ]);
    expect(JSON.stringify(calls)).not.toContain(PREPARATION_ID);
    await expect(api.authorize('not-a-uuid', 'allow')).rejects.toEqual({
      code: 'INVALID_INPUT',
    });
    await expect(api.discard('not-a-uuid')).rejects.toEqual({
      code: 'INVALID_INPUT',
    });
  });

  it('reduces IPC failures to one stable code and exposes no execution method', async () => {
    installTauriMock(() => {
      throw {
        code: 'CONTEXT_TOO_LARGE',
        message: PRIVATE_SENTINEL,
        path: PRIVATE_SENTINEL,
      };
    });
    const api = new TauriBookLearningPreparationApi();
    await expect(
      api.prepare({ kind: 'new', bookId: BOOK_ID, question: QUESTION }),
    ).rejects.toEqual({ code: 'CONTEXT_TOO_LARGE' });
    expect(
      Object.getOwnPropertyNames(Object.getPrototypeOf(api)).sort(),
    ).toEqual(['authorize', 'constructor', 'discard', 'prepare']);
  });
});

function safeSummary() {
  return {
    preparationId: PREPARATION_ID,
    providerDisplayName: 'OpenAI',
    profileDisplayName: 'Book learning profile',
    modelDisplayName: 'Safe model display',
    estimatedInputTokens: 1_200,
    sourceCount: 4,
    citationCount: 2,
    omittedSourceCount: 1,
    riskFlags: [],
    requiresBlockingConfirmation: false,
    expiresAt: '2026-08-06T00:05:00Z',
  } as const;
}
