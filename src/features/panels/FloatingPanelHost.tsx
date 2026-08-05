import { useEffect, useRef, useState } from 'react';

import {
  useLearningRequestActions,
  useLearningRequestSnapshot,
} from '../learning/LearningRequestProvider';
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
}

/** Structural host only. Answer rendering and panel commands are intentionally Task 5 work. */
export function FloatingPanelHost({
  store: suppliedStore,
}: FloatingPanelHostProps) {
  const requestSnapshot = useLearningRequestSnapshot();
  const requestActions = useLearningRequestActions();
  const [store] = useState(() => suppliedStore ?? new PanelStore());
  const [panels, setPanels] = useState(() => store.snapshot());
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
      store.ensureRequest(request.requestId);
      if (request.conversationId) {
        store.completeRequest(request.requestId, request.conversationId);
      }
    }
  }, [requestSnapshot, store]);
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
            onFollowup={(question) => {
              const request = requestSnapshot.requests.find(
                (current) => current.requestId === panel.requestId,
              );
              if (!request?.conversationId || !requestActions) return;
              void requestActions.startFollowup(
                request.conversationId,
                question,
                Object.freeze({
                  action: 'continue',
                  selectionLabel:
                    request.presentation?.selectionLabel ?? 'Learning request',
                  provider: request.presentation?.provider ?? 'Current profile',
                  model: request.presentation?.model ?? 'Current model',
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
  onFollowup,
  onStop,
  onPointerStart,
}: {
  panel: FloatingPanel;
  store: PanelStore;
  request:
    | ReturnType<typeof useLearningRequestSnapshot>['requests'][number]
    | undefined;
  onFollowup(question: string): void;
  onStop(): void;
  onPointerStart(
    handle: ResizeHandle | 'move',
    event: React.PointerEvent,
  ): void;
}) {
  const rect = panelRect(panel.geometry, {
    width: window.innerWidth,
    height: window.innerHeight,
  });
  if (!request) return null;
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
        request={request}
        onCollapse={() => store.setCollapsed(panel.id, !panel.collapsed)}
        onDragStart={(event) => onPointerStart('move', event)}
        onFollowup={onFollowup}
        onHide={() => store.hide(panel.id)}
        onStop={onStop}
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
