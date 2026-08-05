import { useEffect, useRef } from 'react';

import type { AnnotationMarker } from '../contracts';

interface OverlappingMarkerMenuProps {
  readonly markers: readonly AnnotationMarker[];
  readonly returnFocus: HTMLElement | null;
  onActivate(marker: AnnotationMarker): void;
  onClose(): void;
}

export function OverlappingMarkerMenu({
  markers,
  returnFocus,
  onActivate,
  onClose,
}: OverlappingMarkerMenuProps) {
  const firstItem = useRef<HTMLButtonElement>(null);
  const closeRef = useRef(onClose);
  useEffect(() => {
    closeRef.current = onClose;
  }, [onClose]);
  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.preventDefault();
      closeRef.current();
    };
    firstItem.current?.focus();
    window.addEventListener('keydown', closeOnEscape);
    return () => {
      window.removeEventListener('keydown', closeOnEscape);
      returnFocus?.focus();
    };
  }, [returnFocus]);
  return (
    <section
      aria-label="Overlapping markers"
      aria-modal="true"
      className="overlapping-marker-menu"
      role="dialog"
    >
      <div className="overlapping-marker-menu-title">
        <strong>{markers.length} markers at this location</strong>
        <button
          aria-label="Close overlapping markers"
          onClick={onClose}
          type="button"
        >
          ×
        </button>
      </div>
      <ul>
        {markers.map((marker, index) => (
          <li key={marker.id}>
            <button
              ref={index === 0 ? firstItem : undefined}
              data-marker-kind={marker.kind}
              onClick={() => onActivate(marker)}
              type="button"
            >
              <span aria-hidden="true">
                {marker.kind === 'ai_conversation' ? 'AI' : 'N'}
              </span>{' '}
              {marker.label}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
