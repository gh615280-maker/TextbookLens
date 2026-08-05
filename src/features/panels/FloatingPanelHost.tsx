import { useEffect, useRef, useState } from 'react';

import { useLearningRequestSnapshot } from '../learning/LearningRequestProvider';
import {
  movePanel,
  DebouncedPanelGeometryWriter,
  localPanelGeometryPreference,
  panelRect,
  resizePanel,
  type ResizeHandle,
} from './panel-geometry';
import { PanelStore, type FloatingPanel } from './panel-store';

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
  onPointerStart,
}: {
  panel: FloatingPanel;
  onPointerStart(
    handle: ResizeHandle | 'move',
    event: React.PointerEvent,
  ): void;
}) {
  const rect = panelRect(panel.geometry, {
    width: window.innerWidth,
    height: window.innerHeight,
  });
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
      <div
        aria-label="Move learning panel"
        onPointerDown={(event) => onPointerStart('move', event)}
      >
        Learning request
      </div>
      {HANDLES.map((handle) => (
        <button
          key={handle}
          aria-label={`Resize ${handle}`}
          data-resize-handle={handle}
          type="button"
          onPointerDown={(event) => onPointerStart(handle, event)}
        />
      ))}
    </section>
  );
}
