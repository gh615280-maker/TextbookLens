import { act, cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { TeachingApi, TeachingTestEvent } from './api';
import { TeachingTestPanel } from './TeachingTestPanel';

function fakeApi(onEvent: {
  current?: (event: TeachingTestEvent) => void;
}): TeachingApi {
  return {
    getInstruction: vi.fn(),
    updateInstruction: vi.fn(),
    hasAvailableLearningProfile: vi.fn(async () => true),
    startTest: vi.fn(async () => {}),
    cancelTest: vi.fn(async () => {}),
    listenTest: vi.fn(async (handler) => {
      onEvent.current = handler;
      return () => {};
    }),
  };
}

describe('TeachingTestPanel', () => {
  afterEach(() => cleanup());

  it('keeps one active request and ignores late events after completion', async () => {
    const listener: { current?: (event: TeachingTestEvent) => void } = {};
    const api = fakeApi(listener);
    const user = userEvent.setup();
    render(
      <TeachingTestPanel
        api={api}
        enabled
        instruction="Unsaved current draft"
      />,
    );
    await user.click(screen.getByRole('button', { name: 'Show test' }));
    await user.type(
      screen.getByLabelText('Test question'),
      'Synthetic question',
    );
    await user.click(
      screen.getByRole('button', { name: 'Run temporary test' }),
    );

    const startTest = api.startTest as ReturnType<typeof vi.fn>;
    const request = startTest.mock.calls[0]?.[0];
    if (!request) throw new Error('expected a temporary test request');
    expect(request?.instruction).toBe('Unsaved current draft');
    expect(request?.question).toBe('Synthetic question');
    await act(async () => {
      listener.current?.({
        requestId: request.requestId,
        type: 'text_delta',
        text: 'Synthetic answer',
      });
      listener.current?.({
        requestId: request.requestId,
        type: 'completed',
      });
      listener.current?.({
        requestId: 'late-request',
        type: 'text_delta',
        text: ' must not appear',
      });
    });

    expect(screen.getByTestId('teaching-test-stage')).toHaveTextContent(
      'completed',
    );
    expect(screen.getByLabelText('Temporary test answer')).toHaveTextContent(
      'Synthetic answer',
    );
  });

  it('cancels the active request on unmount and when Stop is selected', async () => {
    const listener: { current?: (event: TeachingTestEvent) => void } = {};
    const api = fakeApi(listener);
    const user = userEvent.setup();
    const view = render(
      <TeachingTestPanel api={api} enabled instruction="Draft" />,
    );
    await user.click(screen.getByRole('button', { name: 'Show test' }));
    await user.type(screen.getByLabelText('Test question'), 'Question');
    await user.click(
      screen.getByRole('button', { name: 'Run temporary test' }),
    );
    await user.click(screen.getByRole('button', { name: 'Stop test' }));
    expect(api.cancelTest).toHaveBeenCalledTimes(1);
    view.unmount();
    expect(api.cancelTest).toHaveBeenCalledTimes(1);
  });
});
