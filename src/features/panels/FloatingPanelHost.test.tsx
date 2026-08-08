import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { PropsWithChildren } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { LanguageContext } from '../../app/LanguageProvider';
import { formatMessage } from '../../lib/i18n';
import { LearningRequestProvider } from '../learning/LearningRequestProvider';
import { LearningRequestStore } from '../learning/learning-request-store';
import {
  ConversationPanelOwner,
  ConversationPanelStore,
  type ConversationHistory,
} from '../history/conversation-api';
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
      <EnglishLanguage>
        <LearningRequestProvider store={requests} api={learningApi}>
          <FloatingPanelHost store={panels} />
        </LearningRequestProvider>
      </EnglishLanguage>,
    );
    expect(screen.getByLabelText('Learning request')).toBeInTheDocument();
    const beforeMove = panels.snapshot().panels[0].geometry.xRatio;
    fireEvent.keyDown(screen.getByLabelText('Move learning panel'), {
      key: 'ArrowLeft',
    });
    expect(panels.snapshot().panels[0].geometry.xRatio).toBeLessThan(
      beforeMove,
    );
    const before = panels.snapshot().panels[0].geometry.widthPx;
    fireEvent.keyDown(
      screen.getByLabelText('Resize learning panel right edge'),
      {
        key: 'ArrowRight',
      },
    );
    expect(panels.snapshot().panels[0].geometry.widthPx).toBeGreaterThan(
      before,
    );
    const afterKeyboard = panels.snapshot().panels[0].geometry.widthPx;
    fireEvent.pointerDown(
      screen.getByLabelText('Resize learning panel right edge'),
      {
        clientX: 400,
        clientY: 300,
      },
    );
    fireEvent.pointerMove(window, { clientX: 448, clientY: 300 });
    fireEvent.pointerUp(window);
    expect(panels.snapshot().panels[0].geometry.widthPx).toBeGreaterThan(
      afterKeyboard,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    expect(learningApi.cancel).toHaveBeenCalledWith(
      '11111111-1111-4111-8111-111111111111',
    );
    panels.hide(panels.snapshot().panels[0].id);
    rerender(
      <EnglishLanguage>
        <LearningRequestProvider store={requests} api={learningApi}>
          <FloatingPanelHost store={panels} />
        </LearningRequestProvider>
      </EnglishLanguage>,
    );
    expect(screen.queryByLabelText('Learning request')).not.toBeInTheDocument();
    expect(panels.snapshot().panels).toHaveLength(1);
    expect(learningApi.cancel).toHaveBeenCalledTimes(1);
  });

  it('re-clamps a visible panel after the display viewport shrinks', async () => {
    const originalWidth = window.innerWidth;
    const originalHeight = window.innerHeight;
    Object.defineProperty(window, 'innerWidth', {
      configurable: true,
      value: 1440,
    });
    Object.defineProperty(window, 'innerHeight', {
      configurable: true,
      value: 900,
    });
    const requests = new LearningRequestStore();
    requests.applySnapshot({
      requestId: '12121212-1212-4212-8212-121212121212',
      conversationId: null,
      status: 'streaming',
      text: '',
      usage: null,
      safeError: null,
      lastSeq: 1,
    });
    const { unmount } = render(
      <EnglishLanguage>
        <LearningRequestProvider store={requests} api={api()}>
          <FloatingPanelHost store={new PanelStore()} />
        </LearningRequestProvider>
      </EnglishLanguage>,
    );

    Object.defineProperty(window, 'innerWidth', {
      configurable: true,
      value: 320,
    });
    Object.defineProperty(window, 'innerHeight', {
      configurable: true,
      value: 240,
    });
    fireEvent(window, new Event('resize'));

    await waitFor(() =>
      expect(screen.getByLabelText('Learning request')).toHaveStyle({
        left: '0px',
        top: '0px',
        width: '320px',
        height: '240px',
      }),
    );
    expect(screen.getByLabelText('Move learning panel')).toBeVisible();
    unmount();
    Object.defineProperty(window, 'innerWidth', {
      configurable: true,
      value: originalWidth,
    });
    Object.defineProperty(window, 'innerHeight', {
      configurable: true,
      value: originalHeight,
    });
  });

  it('keeps textbook-level requests on the overview instead of creating reader panels', () => {
    const requests = new LearningRequestStore();
    const panels = new PanelStore();
    const requestId = '99999999-9999-4999-8999-999999999999';
    requests.applySnapshot({
      requestId,
      conversationId: null,
      status: 'streaming',
      text: 'Inline overview answer',
      usage: null,
      safeError: null,
      lastSeq: 1,
    });
    requests.setPresentation(requestId, {
      action: 'ask',
      selectionLabel: 'Book question',
      provider: 'Synthetic provider',
      model: 'Synthetic model',
      bookId: '11111111-1111-4111-8111-111111111111',
    });

    render(
      <EnglishLanguage>
        <LearningRequestProvider store={requests} api={api()}>
          <FloatingPanelHost store={panels} />
        </LearningRequestProvider>
      </EnglishLanguage>,
    );

    expect(screen.queryByLabelText('Learning request')).not.toBeInTheDocument();
    expect(panels.snapshot().panels).toHaveLength(0);
  });

  it('loads immutable history on demand and removes the panel only after atomic delete succeeds', async () => {
    const deletion = deferred<void>();
    const history = conversation();
    const historyStore = new ConversationPanelStore({
      get: vi.fn(async () => history),
      delete: vi.fn(() => deletion.promise),
    });
    const owner = new ConversationPanelOwner(history.bookId);
    await historyStore.open(
      history.bookId,
      history.id,
      history.annotationId,
      owner,
    );
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    render(
      <EnglishLanguage>
        <LearningRequestProvider store={new LearningRequestStore()} api={api()}>
          <FloatingPanelHost
            store={new PanelStore()}
            historyStore={historyStore}
          />
        </LearningRequestProvider>
      </EnglishLanguage>,
    );

    expect(await screen.findByText(/immutable-model-v1/u)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    expect(confirm).toHaveBeenCalledOnce();
    expect(screen.getByLabelText('Learning request')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Delete' })).toBeDisabled();
    deletion.resolve();
    await waitFor(() =>
      expect(
        screen.queryByLabelText('Learning request'),
      ).not.toBeInTheDocument(),
    );
    confirm.mockRestore();
  });

  it('requires explicit Stop and a terminal request before history deletion is enabled', async () => {
    const history = conversation();
    const historyStore = new ConversationPanelStore({
      get: vi.fn(async () => history),
      delete: vi.fn(async () => {}),
    });
    await historyStore.open(
      history.bookId,
      history.id,
      history.annotationId,
      new ConversationPanelOwner(history.bookId),
    );
    const requests = new LearningRequestStore();
    const requestId = '88888888-8888-4888-8888-888888888888';
    requests.applySnapshot({
      requestId,
      conversationId: null,
      status: 'streaming',
      text: 'partial',
      usage: null,
      safeError: null,
      lastSeq: 1,
    });
    requests.setTargetConversation(requestId, history.id);
    const learningApi = api();
    render(
      <EnglishLanguage>
        <LearningRequestProvider store={requests} api={learningApi}>
          <FloatingPanelHost
            store={new PanelStore()}
            historyStore={historyStore}
          />
        </LearningRequestProvider>
      </EnglishLanguage>,
    );
    expect(
      await screen.findByRole('button', { name: 'Delete' }),
    ).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    expect(learningApi.cancel).toHaveBeenCalledWith(requestId);
    expect(historyStore.snapshot().conversations).toHaveLength(1);
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

function conversation(): ConversationHistory {
  return {
    id: '22222222-2222-4222-8222-222222222222',
    bookId: '11111111-1111-4111-8111-111111111111',
    annotationId: '33333333-3333-4333-8333-333333333333',
    sectionId: '44444444-4444-4444-8444-444444444444',
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
        sectionId: '44444444-4444-4444-8444-444444444444',
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
        providerId: '55555555-5555-4555-8555-555555555555',
        modelId: 'immutable-model-v1',
        citations: [],
        createdAt: '2026-08-05T00:00:00.000Z',
      },
    ],
    createdAt: '2026-08-05T00:00:00.000Z',
    updatedAt: '2026-08-05T00:00:00.000Z',
  };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
