import { useEffect, useRef, useState, useSyncExternalStore } from 'react';

import {
  useLearningRequestActions,
  useLearningRequestSnapshot,
} from '../learning/LearningRequestProvider';
import type { LearningRequestView } from '../learning/learning-request-store';
import {
  ConversationPanelStore,
  conversationPanels,
  type ConversationHistory,
} from '../history/conversation-api';
import {
  movePanel,
  DebouncedPanelGeometryWriter,
  localPanelGeometryPreference,
  panelRect,
  resizePanel,
  type ResizeHandle,
} from './panel-geometry';
import { PanelStore, type FloatingPanel } from './panel-store';
import { FloatingAnswerPanel } from './FloatingAnswerPanel';
import './floating-panels.css';

const HANDLES: readonly ResizeHandle[] = [
  'n',
  'ne',
  'e',
  'se',
  's',
  'sw',
  'w',
  'nw',
];

interface FloatingPanelHostProps {
  store?: PanelStore;
  historyStore?: ConversationPanelStore;
}

/** Structural host only. Answer rendering and panel commands are intentionally Task 5 work. */
export function FloatingPanelHost({
  store: suppliedStore,
  historyStore: suppliedHistoryStore,
}: FloatingPanelHostProps) {
  const requestSnapshot = useLearningRequestSnapshot();
  const requestActions = useLearningRequestActions();
  const [store] = useState(() => suppliedStore ?? new PanelStore());
  const [historyStore] = useState(
    () => suppliedHistoryStore ?? conversationPanels,
  );
  const historySnapshot = useSyncExternalStore(
    (listener) => historyStore.subscribe(listener),
    () => historyStore.snapshot(),
    () => historyStore.snapshot(),
  );
  const [panels, setPanels] = useState(() => store.snapshot());
  const [deleteFailures, setDeleteFailures] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const openedHistory = useRef(new Map<string, ConversationHistory>());
  const refreshedRequests = useRef(new Set<string>());
  const interaction = useRef<{
    id: string;
    handle: ResizeHandle | 'move';
    x: number;
    y: number;
  } | null>(null);
  const [preferenceWriter] = useState(
    () => new DebouncedPanelGeometryWriter(localPanelGeometryPreference()),
  );

  useEffect(() => store.subscribe(() => setPanels(store.snapshot())), [store]);
  useEffect(() => () => preferenceWriter.dispose(), [preferenceWriter]);
  useEffect(() => {
    const top = panels.panels.at(-1);
    if (top) preferenceWriter.save(top.geometry);
  }, [panels, preferenceWriter]);
  useEffect(() => {
    for (const request of requestSnapshot.requests) {
      if (request.presentation?.bookId) {
        store.removeRequest(request.requestId);
        continue;
      }
      const owner = request.targetConversationId ?? request.conversationId;
      if (owner && historyStore.isDeleted(owner)) continue;
      if (request.targetConversationId) {
        store.attachRequestToConversation(
          request.requestId,
          request.targetConversationId,
        );
      } else {
        store.ensureRequest(request.requestId);
      }
      if (request.conversationId) {
        store.completeRequest(request.requestId, request.conversationId);
      }
      if (
        request.status === 'completed' &&
        request.targetConversationId &&
        !refreshedRequests.current.has(request.requestId)
      ) {
        refreshedRequests.current.add(request.requestId);
        void historyStore.refresh(request.targetConversationId).catch(() => {
          // Durable history remains unchanged when a safe reload fails.
        });
      }
    }
  }, [historyStore, requestSnapshot, store]);
  useEffect(() => {
    const present = new Set<string>();
    for (const history of historySnapshot.conversations) {
      present.add(history.id);
      if (openedHistory.current.get(history.id) !== history) {
        openedHistory.current.set(history.id, history);
        store.openConversation(history.id);
      }
    }
    for (const [conversationId] of openedHistory.current) {
      if (
        !present.has(conversationId) &&
        historyStore.isDeleted(conversationId)
      ) {
        openedHistory.current.delete(conversationId);
        store.removeConversation(conversationId);
      }
    }
  }, [historySnapshot, historyStore, store]);
  useEffect(() => {
    const move = (event: PointerEvent) => {
      const current = interaction.current;
      if (!current) return;
      const panel = store.snapshot().panels.find(({ id }) => id === current.id);
      if (!panel) return;
      const viewport = { width: window.innerWidth, height: window.innerHeight };
      const delta = {
        x: event.clientX - current.x,
        y: event.clientY - current.y,
      };
      store.setGeometry(
        panel.id,
        current.handle === 'move'
          ? movePanel(panel.geometry, delta, viewport)
          : resizePanel(panel.geometry, current.handle, delta, viewport),
      );
      current.x = event.clientX;
      current.y = event.clientY;
    };
    const stop = () => {
      interaction.current = null;
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', stop);
    return () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', stop);
    };
  }, [store]);

  return (
    <div aria-live="off" data-floating-panel-host="true">
      {panels.panels
        .filter((panel) => !panel.hidden)
        .map((panel) => (
          <Panel
            key={panel.id}
            panel={panel}
            store={store}
            request={requestSnapshot.requests.find(
              (request) => request.requestId === panel.requestId,
            )}
            history={historySnapshot.conversations.find(
              (history) => history.id === panel.conversationId,
            )}
            deletePending={
              panel.conversationId
                ? [...historySnapshot.pendingDeletes].some((key) =>
                    key.endsWith(`:${panel.conversationId}`),
                  )
                : false
            }
            deleteFailed={
              panel.conversationId
                ? deleteFailures.has(panel.conversationId)
                : false
            }
            onFollowup={(question) => {
              const request = requestSnapshot.requests.find(
                (current) => current.requestId === panel.requestId,
              );
              const history = historySnapshot.conversations.find(
                (current) => current.id === panel.conversationId,
              );
              const conversationId =
                panel.conversationId ?? request?.conversationId;
              if (!conversationId || !requestActions) return;
              const historicalAssistant = history
                ? [...history.messages]
                    .reverse()
                    .find((message) => message.role === 'assistant')
                : undefined;
              void requestActions.startFollowup(
                conversationId,
                question,
                Object.freeze({
                  action: 'continue',
                  selectionLabel:
                    request?.presentation?.selectionLabel ??
                    history?.selectedText ??
                    'Visual region',
                  provider:
                    request?.presentation?.provider ??
                    historicalAssistant?.providerId ??
                    'Historical provider',
                  model:
                    request?.presentation?.model ??
                    historicalAssistant?.modelId ??
                    'Historical model',
                }),
              );
            }}
            onStop={() => {
              const request = requestSnapshot.requests.find(
                (current) => current.requestId === panel.requestId,
              );
              if (
                request?.status === 'preparing' ||
                request?.status === 'streaming'
              ) {
                void requestActions?.cancel(request.requestId);
              }
            }}
            onDelete={(history) => {
              if (!window.confirm('Delete this conversation and its marker?'))
                return;
              setDeleteFailures((current) => {
                const next = new Set(current);
                next.delete(history.id);
                return next;
              });
              void historyStore.delete(history).catch(() => {
                setDeleteFailures((current) =>
                  new Set(current).add(history.id),
                );
              });
            }}
            onPointerStart={(handle, event) => {
              event.preventDefault();
              interaction.current = {
                id: panel.id,
                handle,
                x: event.clientX,
                y: event.clientY,
              };
              store.reopen(panel.id);
            }}
          />
        ))}
    </div>
  );
}

function Panel({
  panel,
  store,
  request,
  history,
  deletePending,
  deleteFailed,
  onFollowup,
  onStop,
  onDelete,
  onPointerStart,
}: {
  panel: FloatingPanel;
  store: PanelStore;
  request:
    | ReturnType<typeof useLearningRequestSnapshot>['requests'][number]
    | undefined;
  history: ConversationHistory | undefined;
  deletePending: boolean;
  deleteFailed: boolean;
  onFollowup(question: string): void;
  onStop(): void;
  onDelete(history: ConversationHistory): void;
  onPointerStart(
    handle: ResizeHandle | 'move',
    event: React.PointerEvent,
  ): void;
}) {
  const rect = panelRect(panel.geometry, {
    width: window.innerWidth,
    height: window.innerHeight,
  });
  const visibleRequest = request ?? (history ? historyRequest(history) : null);
  if (!visibleRequest) return null;
  return (
    <section
      aria-label="Learning request"
      data-panel-id={panel.id}
      style={{
        height: rect.height,
        left: rect.x,
        position: 'fixed',
        top: rect.y,
        width: rect.width,
        zIndex: panel.zIndex,
      }}
    >
      <FloatingAnswerPanel
        collapsed={panel.collapsed}
        request={visibleRequest}
        history={history}
        deleteDisabled={deletePending}
        deleteError={deleteFailed}
        onCollapse={() => store.setCollapsed(panel.id, !panel.collapsed)}
        onDragStart={(event) => onPointerStart('move', event)}
        onFollowup={onFollowup}
        onHide={() => store.hide(panel.id)}
        onStop={onStop}
        onDelete={history ? () => onDelete(history) : undefined}
      />
      {HANDLES.map((handle) => (
        <button
          key={handle}
          aria-label={`Resize ${handle}`}
          data-resize-handle={handle}
          type="button"
          onKeyDown={(event) => {
            const step = event.shiftKey ? 32 : 16;
            const keys = {
              ArrowUp: { x: 0, y: -step },
              ArrowDown: { x: 0, y: step },
              ArrowLeft: { x: -step, y: 0 },
              ArrowRight: { x: step, y: 0 },
            } as const;
            const delta = keys[event.key as keyof typeof keys];
            if (!delta) return;
            event.preventDefault();
            const viewport = {
              width: window.innerWidth,
              height: window.innerHeight,
            };
            store.setGeometry(
              panel.id,
              resizePanel(panel.geometry, handle, delta, viewport),
            );
          }}
          onPointerDown={(event) => onPointerStart(handle, event)}
        />
      ))}
    </section>
  );
}

function historyRequest(history: ConversationHistory): LearningRequestView {
  const assistant = [...history.messages]
    .reverse()
    .find((message) => message.role === 'assistant');
  return Object.freeze({
    requestId: `history:${history.id}`,
    conversationId: history.id,
    status: 'completed',
    text: '',
    usage: null,
    safeError: null,
    lastSeq: 0,
    targetConversationId: null,
    presentation: Object.freeze({
      action: assistant?.action ?? 'learning',
      selectionLabel: history.selectedText ?? 'Visual region',
      provider: assistant?.providerId ?? 'Historical provider',
      model: assistant?.modelId ?? 'Historical model',
    }),
  });
}
