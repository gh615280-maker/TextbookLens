import { render, waitFor } from '@testing-library/react';
import { useEffect } from 'react';
import { describe, expect, it, vi } from 'vitest';

import {
  LearningRequestProvider,
  useLearningSurfacePort,
} from './LearningRequestProvider';
import { LearningRequestStore } from './learning-request-store';

const requestId = '11111111-1111-4111-8111-111111111111';

describe('LearningRequestProvider', () => {
  it('starts then subscribes globally, and teardown does not cancel the request', async () => {
    let deliver:
      | ((event: {
          requestId: string;
          seq: number;
          event: { type: 'text_delta'; text: string };
        }) => void)
      | undefined;
    const api = {
      start: vi.fn(async () => initial()),
      subscribe: vi.fn(async (_id, _seq, onEvent) => {
        deliver = onEvent;
        return { snapshot: initial(), unsubscribe: vi.fn() };
      }),
      cancel: vi.fn(),
      startFollowup: vi.fn(async () => initial()),
    };
    const store = new LearningRequestStore();
    const { unmount } = render(
      <LearningRequestProvider api={api} store={store}>
        <Starter />
      </LearningRequestProvider>,
    );
    await waitFor(() =>
      expect(api.subscribe).toHaveBeenCalledWith(
        requestId,
        0,
        expect.any(Function),
      ),
    );
    deliver?.({
      requestId,
      seq: 1,
      event: { type: 'text_delta', text: 'safe' },
    });
    expect(store.get(requestId)?.text).toBe('safe');
    unmount();
    expect(api.cancel).not.toHaveBeenCalled();
  });
});

function Starter() {
  const surface = useLearningSurfacePort();
  useEffect(() => {
    if (!surface) return;
    void surface.handoff({
      preparationId: '33333333-3333-4333-8333-333333333333',
      summary: {} as never,
      action: 'explain',
      selectionLabel: 'Synthetic selection',
    });
  }, [surface]);
  return null;
}

function initial() {
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
