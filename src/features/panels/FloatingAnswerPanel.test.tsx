import { fireEvent, render, screen } from '@testing-library/react';
import type { PropsWithChildren } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { LanguageContext } from '../../app/LanguageProvider';
import { formatMessage } from '../../lib/i18n';
import { FloatingAnswerPanel } from './FloatingAnswerPanel';

const request = {
  requestId: '11111111-1111-4111-8111-111111111111',
  conversationId: null,
  status: 'streaming' as const,
  text: 'partial',
  usage: null,
  safeError: null,
  lastSeq: 1,
  targetConversationId: null,
  presentation: {
    action: 'explain',
    selectionLabel: 'Original selection',
    provider: 'Current',
    model: 'model',
  },
};

describe('FloatingAnswerPanel', () => {
  it('keeps hide and collapse separate from Stop, without stealing focus', () => {
    const onHide = vi.fn();
    const onStop = vi.fn();
    render(
      <EnglishLanguage>
        <FloatingAnswerPanel
          collapsed={false}
          request={request}
          onCollapse={vi.fn()}
          onDragStart={vi.fn()}
          onFollowup={vi.fn()}
          onHide={onHide}
          onMoveKeyDown={vi.fn()}
          onStop={onStop}
        />
      </EnglishLanguage>,
    );
    const stop = screen.getByRole('button', { name: 'Stop' });
    stop.focus();
    fireEvent.click(screen.getByRole('button', { name: 'Hide' }));
    expect(onHide).toHaveBeenCalledOnce();
    expect(onStop).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(stop);
    fireEvent.click(stop);
    expect(onStop).toHaveBeenCalledOnce();
    expect(screen.getByRole('button', { name: 'Retry' })).toBeDisabled();
  });
});

function EnglishLanguage({ children }: PropsWithChildren) {
  return (
    <LanguageContext.Provider
      value={{
        uiLanguage: 'en',
        isLoading: false,
        statusMessage: null,
        switchLanguage: async () => {},
        message: (key, values) => formatMessage('en', key, values),
      }}
    >
      {children}
    </LanguageContext.Provider>
  );
}
