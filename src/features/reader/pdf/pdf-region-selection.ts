import type { NormalizedRect } from '../../../lib/generated/document';
import type { PdfRegionInstructionCode } from '../contracts';

export const PDF_REGION_MIN_CSS_PIXELS = 8;

export interface PdfRegionRect {
  page: number;
  rect: NormalizedRect;
}

export class PdfRegionSelectionError extends Error {
  constructor(readonly instructionCode: PdfRegionInstructionCode) {
    super(instructionCode);
    this.name = 'PdfRegionSelectionError';
  }
}

export function pageRelativeRect(
  page: number,
  pageBounds: DOMRect | Pick<DOMRect, 'left' | 'top' | 'width' | 'height'>,
  start: { x: number; y: number },
  end: { x: number; y: number },
): PdfRegionRect {
  const left = Math.max(pageBounds.left, Math.min(start.x, end.x));
  const top = Math.max(pageBounds.top, Math.min(start.y, end.y));
  const right = Math.min(
    pageBounds.left + pageBounds.width,
    Math.max(start.x, end.x),
  );
  const bottom = Math.min(
    pageBounds.top + pageBounds.height,
    Math.max(start.y, end.y),
  );
  if (
    pageBounds.width <= 0 ||
    pageBounds.height <= 0 ||
    right - left < PDF_REGION_MIN_CSS_PIXELS ||
    bottom - top < PDF_REGION_MIN_CSS_PIXELS
  )
    throw new PdfRegionSelectionError('pdf_region_too_small');
  return {
    page,
    rect: {
      x: stableRatio(left - pageBounds.left, pageBounds.width),
      y: stableRatio(top - pageBounds.top, pageBounds.height),
      width: stableRatio(right - left, pageBounds.width),
      height: stableRatio(bottom - top, pageBounds.height),
    },
  };
}

export function pageAtPoint(root: HTMLElement, x: number, y: number) {
  return [...root.querySelectorAll<HTMLElement>('[data-page-number]')].find(
    (page) => {
      const bounds = page.getBoundingClientRect();
      return (
        x >= bounds.left &&
        x <= bounds.right &&
        y >= bounds.top &&
        y <= bounds.bottom
      );
    },
  );
}

/** Converts a displayed-page rect to stable, unrotated PDF page space. */
export function normalizeForPdfRotation(
  rect: NormalizedRect,
  rotation: number,
): NormalizedRect {
  let result: NormalizedRect;
  switch (((rotation % 360) + 360) % 360) {
    case 0:
      result = { ...rect };
      break;
    case 90:
      result = {
        x: rect.y,
        y: 1 - rect.x - rect.width,
        width: rect.height,
        height: rect.width,
      };
      break;
    case 180:
      result = {
        x: 1 - rect.x - rect.width,
        y: 1 - rect.y - rect.height,
        width: rect.width,
        height: rect.height,
      };
      break;
    case 270:
      result = {
        x: 1 - rect.y - rect.height,
        y: rect.x,
        width: rect.height,
        height: rect.width,
      };
      break;
    default:
      throw new PdfRegionSelectionError('pdf_region_unavailable');
  }
  return {
    x: stableNumber(result.x),
    y: stableNumber(result.y),
    width: stableNumber(result.width),
    height: stableNumber(result.height),
  };
}

const NORMALIZED_PRECISION = 1_000_000;
function stableRatio(value: number, total: number): number {
  return stableNumber(value / total);
}
function stableNumber(value: number): number {
  return Math.round(value * NORMALIZED_PRECISION) / NORMALIZED_PRECISION;
}
