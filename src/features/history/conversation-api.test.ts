import { describe, expect, it, vi } from 'vitest';

import {
  ConversationPanelOwner,
  ConversationPanelStore,
  type ConversationApi,
  type ConversationHistory,
} from './conversation-api';

const BOOK = '11111111-1111-4111-8111-111111111111';
const CONVERSATION = '22222222-2222-4222-8222-222222222222';
const ANNOTATION = '33333333-3333-4333-8333-333333333333';
const SECTION = '44444444-4444-4444-8444-444444444444';
const PROVIDER = '55555555-5555-4555-8555-555555555555';

describe('ConversationPanelStore', () => {
  it('drops a late marker load when its reader owner is disposed', async () => {
    const pending = deferred<ConversationHistory>();
    const api = mockApi(() => pending.promise);
    const store = new ConversationPanelStore(api);
    const owner = new ConversationPanelOwner(BOOK);
    const opened = store.open(BOOK, CONVERSATION, ANNOTATION, owner);
    owner.dispose();
    pending.resolve(history());
    await expect(opened).resolves.toBe(false);
    expect(store.snapshot().conversations).toHaveLength(0);
  });

  it('keeps history visible until atomic delete succeeds and blocks late resurrection', async () => {
    const deletion = deferred<void>();
    const api = mockApi(
      async () => history(),
      () => deletion.promise,
    );
    const store = new ConversationPanelStore(api);
    const owner = new ConversationPanelOwner(BOOK);
    await expect(
      store.open(BOOK, CONVERSATION, ANNOTATION, owner),
    ).resolves.toBe(true);
    const loaded = store.snapshot().conversations[0];
    expect(Object.isFrozen(loaded.messages[1])).toBe(true);
    expect(loaded.messages[1]).toMatchObject({
      providerId: PROVIDER,
      modelId: 'immutable-model-v1',
    });

    const deleted = store.delete(loaded);
    expect(store.snapshot().conversations).toHaveLength(1);
    expect(store.snapshot().pendingDeletes.size).toBe(1);
    deletion.resolve();
    await deleted;
    expect(store.snapshot().conversations).toHaveLength(0);
    expect(store.isDeleted(CONVERSATION)).toBe(true);
    await expect(
      store.open(BOOK, CONVERSATION, ANNOTATION, owner),
    ).resolves.toBe(false);
    expect(api.get).toHaveBeenCalledTimes(1);
  });

  it('rolls back only pending UI state when backend deletion fails', async () => {
    const api = mockApi(
      async () => history(),
      async () => {
        throw { code: 'DATABASE_ERROR' };
      },
    );
    const store = new ConversationPanelStore(api);
    const owner = new ConversationPanelOwner(BOOK);
    await store.open(BOOK, CONVERSATION, ANNOTATION, owner);
    await expect(
      store.delete(store.snapshot().conversations[0]),
    ).rejects.toEqual({
      code: 'DATABASE_ERROR',
    });
    expect(store.snapshot().conversations).toHaveLength(1);
    expect(store.snapshot().pendingDeletes.size).toBe(0);
  });
});

function history(): ConversationHistory {
  return {
    id: CONVERSATION,
    bookId: BOOK,
    annotationId: ANNOTATION,
    sectionId: SECTION,
    anchor: {
      kind: 'text',
      selection: {
        locator: {
          format: 'pdf',
          startPage: 1,
          endPage: 1,
          rectsByPage: null,
        },
        quote: { exact: 'Synthetic selection', prefix: '', suffix: '' },
        sectionId: SECTION,
      },
    },
    selectedText: 'Synthetic selection',
    status: 'completed',
    messages: [
      {
        id: '66666666-6666-4666-8666-666666666666',
        ordinal: 0,
        role: 'user',
        action: 'explain',
        content: 'Synthetic question',
        providerId: null,
        modelId: null,
        citations: [],
        createdAt: '2026-08-05T00:00:00.000Z',
      },
      {
        id: '77777777-7777-4777-8777-777777777777',
        ordinal: 1,
        role: 'assistant',
        action: 'explain',
        content: 'Synthetic answer',
        providerId: PROVIDER,
        modelId: 'immutable-model-v1',
        citations: [],
        createdAt: '2026-08-05T00:00:00.000Z',
      },
    ],
    createdAt: '2026-08-05T00:00:00.000Z',
    updatedAt: '2026-08-05T00:00:00.000Z',
  };
}

function mockApi(
  get: () => Promise<ConversationHistory>,
  remove: () => Promise<void> = async () => {},
): ConversationApi & {
  get: ReturnType<typeof vi.fn>;
  delete: ReturnType<typeof vi.fn>;
} {
  return { get: vi.fn(get), delete: vi.fn(remove) };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
