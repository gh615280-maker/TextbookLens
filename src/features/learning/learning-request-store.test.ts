import { describe, expect, it } from 'vitest';

import { LearningRequestStore } from './learning-request-store';

const requestId = '11111111-1111-4111-8111-111111111111';
const conversationId = '22222222-2222-4222-8222-222222222222';

describe('LearningRequestStore', () => {
  it('accepts only monotonic events and ignores post-terminal delivery', () => {
    const store = new LearningRequestStore();
    store.applySnapshot(snapshot());
    expect(
      store.applyEvent(event(1, { type: 'text_delta', text: 'Answer' })),
    ).toBe(true);
    expect(
      store.applyEvent(event(1, { type: 'text_delta', text: ' duplicate' })),
    ).toBe(false);
    expect(
      store.applyEvent(event(2, { type: 'completed', conversationId })),
    ).toBe(true);
    expect(
      store.applyEvent(event(3, { type: 'text_delta', text: ' late' })),
    ).toBe(false);
    expect(store.get(requestId)).toMatchObject({
      text: 'Answer',
      status: 'completed',
      conversationId,
      lastSeq: 2,
    });
  });

  it('does not replace newer state with a late snapshot', () => {
    const store = new LearningRequestStore();
    store.applySnapshot(snapshot());
    store.applyEvent(event(1, { type: 'text_delta', text: 'Fresh' }));
    expect(store.applySnapshot(snapshot())).toBe(false);
    expect(store.get(requestId)?.text).toBe('Fresh');
  });
});

function snapshot() {
  return {
    requestId,
    conversationId: null,
    status: 'preparing' as const,
    text: '',
    usage: null,
    safeError: null,
    lastSeq: 0,
  };
}

function event(
  seq: number,
  value: Parameters<LearningRequestStore['applyEvent']>[0]['event'],
) {
  return { requestId, seq, event: value };
}
