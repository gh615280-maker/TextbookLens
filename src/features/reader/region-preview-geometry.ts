export interface ClientPoint {
  x: number;
  y: number;
}

export interface LocalSelectionRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

/**
 * Converts viewport pointer coordinates into the unscaled CSS coordinate space
 * used by an absolutely positioned preview child.
 */
export function clientSelectionRect(
  container: HTMLElement,
  start: ClientPoint,
  end: ClientPoint,
): LocalSelectionRect {
  const bounds = container.getBoundingClientRect();
  const scaleX = renderedScale(bounds.width, container.offsetWidth);
  const scaleY = renderedScale(bounds.height, container.offsetHeight);
  const left = clamp(Math.min(start.x, end.x), bounds.left, bounds.right);
  const top = clamp(Math.min(start.y, end.y), bounds.top, bounds.bottom);
  const right = clamp(Math.max(start.x, end.x), bounds.left, bounds.right);
  const bottom = clamp(Math.max(start.y, end.y), bounds.top, bounds.bottom);
  return {
    left: (left - bounds.left) / scaleX,
    top: (top - bounds.top) / scaleY,
    width: Math.max(0, right - left) / scaleX,
    height: Math.max(0, bottom - top) / scaleY,
  };
}

/** Maps an iframe-local pointer into an outer container's local CSS space. */
export function iframeClientPointToLocal(
  container: HTMLElement,
  frame: HTMLElement,
  point: ClientPoint,
): ClientPoint {
  const containerBounds = container.getBoundingClientRect();
  const frameBounds = frame.getBoundingClientRect();
  const containerScaleX = renderedScale(
    containerBounds.width,
    container.offsetWidth,
  );
  const containerScaleY = renderedScale(
    containerBounds.height,
    container.offsetHeight,
  );
  const frameScaleX = renderedScale(frameBounds.width, frame.offsetWidth);
  const frameScaleY = renderedScale(frameBounds.height, frame.offsetHeight);
  return {
    x:
      (frameBounds.left + point.x * frameScaleX - containerBounds.left) /
      containerScaleX,
    y:
      (frameBounds.top + point.y * frameScaleY - containerBounds.top) /
      containerScaleY,
  };
}

function renderedScale(renderedSize: number, layoutSize: number): number {
  return renderedSize > 0 && layoutSize > 0 ? renderedSize / layoutSize : 1;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(value, maximum));
}
