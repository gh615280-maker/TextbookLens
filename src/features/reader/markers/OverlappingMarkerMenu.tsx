import { useEffect, useRef } from 'react';

import { useMessage } from '../../../app/LanguageProvider';
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
  const message = useMessage();
  const firstItem = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLElement>(null);
  const closeRef = useRef(onClose);
  useEffect(() => {
    closeRef.current = onClose;
  }, [onClose]);
  useEffect(() => {
    const handleKeyboard = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        closeRef.current();
        return;
      }
      if (event.key !== 'Tab') return;
      const focusable = dialogRef.current?.querySelectorAll<HTMLElement>(
        'button:not([disabled]), [href], [tabindex]:not([tabindex="-1"])',
      );
      const first = focusable?.item(0);
      const last = focusable?.item((focusable?.length ?? 1) - 1);
      if (!first || !last) return;
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    firstItem.current?.focus();
    window.addEventListener('keydown', handleKeyboard);
    return () => {
      window.removeEventListener('keydown', handleKeyboard);
      returnFocus?.focus();
    };
  }, [returnFocus]);
  return (
    <section
      ref={dialogRef}
      aria-label={message('markers.overlap.label')}
      aria-modal="true"
      className="overlapping-marker-menu"
      role="dialog"
    >
      <div className="overlapping-marker-menu-title">
        <strong>
          {message('markers.overlap.title', { count: markers.length })}
        </strong>
        <button
          aria-label={message('markers.overlap.close')}
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
