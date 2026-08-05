import { useEffect } from 'react';

interface RegionSelectionOverlayProps {
  active: boolean;
  instruction: string;
  status: string;
  onCancel(): void;
}

/** Accessible state announcement for adapter-owned, one-shot region dragging. */
export function RegionSelectionOverlay({
  active,
  instruction,
  status,
  onCancel,
}: RegionSelectionOverlayProps) {
  useEffect(() => {
    if (!active) return;
    const cancel = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onCancel();
      }
    };
    window.addEventListener('keydown', cancel);
    return () => window.removeEventListener('keydown', cancel);
  }, [active, onCancel]);
  if (!active) return null;
  return (
    <div className="learning-region-overlay" aria-live="polite" role="status">
      <span>{instruction}</span>
      <span>{status}</span>
      <button type="button" onClick={onCancel}>
        {status}
      </button>
    </div>
  );
}
