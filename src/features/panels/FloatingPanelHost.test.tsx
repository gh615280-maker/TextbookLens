import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { LearningRequestProvider } from '../learning/LearningRequestProvider';
import { LearningRequestStore } from '../learning/learning-request-store';
import { FloatingPanelHost } from './FloatingPanelHost';
import { PanelStore } from './panel-store';

describe('FloatingPanelHost', () => {
  it('hosts visible panels while retaining hidden panels in its internal store', () => {
    const requests = new LearningRequestStore();
    const panels = new PanelStore();
    const learningApi = api();
    requests.applySnapshot({
      requestId: '11111111-1111-4111-8111-111111111111',
      conversationId: null,
      status: 'streaming',
      text: '',
      usage: null,
      safeError: null,
      lastSeq: 1,
    });
    const { rerender } = render(
      <LearningRequestProvider store={requests} api={learningApi}>
        <FloatingPanelHost store={panels} />
      </LearningRequestProvider>,
    );
    expect(screen.getByLabelText('Learning request')).toBeInTheDocument();
    const before = panels.snapshot().panels[0].geometry.widthPx;
    fireEvent.keyDown(screen.getByLabelText('Resize e'), { key: 'ArrowRight' });
    expect(panels.snapshot().panels[0].geometry.widthPx).toBeGreaterThan(
      before,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    expect(learningApi.cancel).toHaveBeenCalledWith(
      '11111111-1111-4111-8111-111111111111',
    );
    panels.hide(panels.snapshot().panels[0].id);
    rerender(
      <LearningRequestProvider store={requests} api={learningApi}>
        <FloatingPanelHost store={panels} />
      </LearningRequestProvider>,
    );
    expect(screen.queryByLabelText('Learning request')).not.toBeInTheDocument();
    expect(panels.snapshot().panels).toHaveLength(1);
    expect(learningApi.cancel).toHaveBeenCalledTimes(1);
  });
});

function api() {
  return {
    start: async () => {
      throw new Error('unused');
    },
    subscribe: async () => {
      throw new Error('unused');
    },
    cancel: vi.fn(async () => {}),
    startFollowup: async () => {
      throw new Error('unused');
    },
  };
}
