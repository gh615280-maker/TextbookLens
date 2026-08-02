import type { NormalizedRect } from '../../../lib/generated/document';
import type { ClientRectLike, PageBounds } from './pdf-selection';
import { denormalizeRects } from './pdf-selection';

export function addPdfRectOverlay(
  container: HTMLElement,
  bounds: PageBounds,
  rects: readonly NormalizedRect[],
): () => void {
  const overlay = document.createElement('div');
  overlay.className = 'pdf-marker-overlay';
  overlay.setAttribute('aria-hidden', 'true');
  for (const rect of denormalizeRects(bounds, rects)) {
    const marker = document.createElement('span');
    marker.className = 'pdf-marker-rect';
    applyRect(marker, rect, bounds);
    overlay.append(marker);
  }
  container.append(overlay);
  return () => overlay.remove();
}

function applyRect(
  element: HTMLElement,
  rect: ClientRectLike,
  bounds: PageBounds,
): void {
  Object.assign(element.style, {
    left: `${rect.left - bounds.left}px`,
    top: `${rect.top - bounds.top}px`,
    width: `${rect.width}px`,
    height: `${rect.height}px`,
  });
}
