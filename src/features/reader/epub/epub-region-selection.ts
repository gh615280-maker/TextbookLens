import type { NormalizedRect } from '../../../lib/generated/document';
import type { EpubRegionInstructionCode } from '../contracts';

export const EPUB_REGION_MIN_CSS_PIXELS = 8;

export interface Point {
  x: number;
  y: number;
}

export interface RectBounds {
  left: number;
  top: number;
  width: number;
  height: number;
}

export class EpubRegionSelectionError extends Error {
  constructor(readonly instructionCode: EpubRegionInstructionCode) {
    super(instructionCode);
    this.name = 'EpubRegionSelectionError';
  }
}

/**
 * Converts coordinates from an EPUB contents viewport into a stable rectangle
 * relative to one CFI-addressable element. The result is independent of the
 * outer iframe position, window scroll, font scale and rendered element size.
 */
export function epubElementRelativeRect(
  elementBounds: RectBounds,
  start: Point,
  end: Point,
): NormalizedRect {
  return relativeRect(elementBounds, start, end);
}

/**
 * Converts host-window coordinates through an iframe before normalizing them
 * to an element. `elementBounds` must be measured inside the iframe viewport.
 */
export function iframeElementRelativeRect(
  frameBounds: Pick<RectBounds, 'left' | 'top'>,
  elementBounds: RectBounds,
  start: Point,
  end: Point,
): NormalizedRect {
  return relativeRect(
    {
      left: frameBounds.left + elementBounds.left,
      top: frameBounds.top + elementBounds.top,
      width: elementBounds.width,
      height: elementBounds.height,
    },
    start,
    end,
  );
}

/** Chooses a conservative block/visual container that EPUB.js can turn into a CFI. */
export function epubRegionContainer(
  target: EventTarget | null,
  document: Document,
): HTMLElement | null {
  if (
    !target ||
    !('nodeType' in target) ||
    (target as Node).nodeType !== Node.ELEMENT_NODE ||
    (target as Element).ownerDocument !== document
  )
    return null;
  const container = (target as Element).closest<HTMLElement>(
    [
      '[data-epub-region-id]',
      '[id]',
      'figure',
      'math',
      'table',
      'pre',
      'blockquote',
      'p',
      'li',
      'h1',
      'h2',
      'h3',
      'h4',
      'h5',
      'h6',
      'img',
      'svg',
      'canvas',
    ].join(','),
  );
  return container && document.body.contains(container) ? container : null;
}

export function elementAtEpubPoint(
  document: Document,
  point: Point,
): Element | null {
  return document.elementFromPoint?.(point.x, point.y) ?? null;
}

function relativeRect(
  bounds: RectBounds,
  start: Point,
  end: Point,
): NormalizedRect {
  if (
    !finiteBounds(bounds) ||
    !finitePoint(start) ||
    !finitePoint(end) ||
    bounds.width <= 0 ||
    bounds.height <= 0
  )
    throw new EpubRegionSelectionError('epub_region_unavailable');
  const left = Math.max(bounds.left, Math.min(start.x, end.x));
  const top = Math.max(bounds.top, Math.min(start.y, end.y));
  const right = Math.min(bounds.left + bounds.width, Math.max(start.x, end.x));
  const bottom = Math.min(bounds.top + bounds.height, Math.max(start.y, end.y));
  if (
    right - left < EPUB_REGION_MIN_CSS_PIXELS ||
    bottom - top < EPUB_REGION_MIN_CSS_PIXELS
  )
    throw new EpubRegionSelectionError('epub_region_too_small');
  return {
    x: stableRatio(left - bounds.left, bounds.width),
    y: stableRatio(top - bounds.top, bounds.height),
    width: stableRatio(right - left, bounds.width),
    height: stableRatio(bottom - top, bounds.height),
  };
}

function finiteBounds(bounds: RectBounds): boolean {
  return [bounds.left, bounds.top, bounds.width, bounds.height].every(
    Number.isFinite,
  );
}
function finitePoint(point: Point): boolean {
  return Number.isFinite(point.x) && Number.isFinite(point.y);
}
const NORMALIZED_PRECISION = 1_000_000;
function stableRatio(value: number, total: number): number {
  return stableNumber(value / total);
}
function stableNumber(value: number): number {
  return Math.round(value * NORMALIZED_PRECISION) / NORMALIZED_PRECISION;
}
