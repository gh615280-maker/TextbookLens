import type { NormalizedRect } from '../../../lib/generated/document';
import type { DocxRegionInstructionCode } from '../contracts';

export const DOCX_REGION_MIN_CSS_PIXELS = 8;

export class DocxRegionSelectionError extends Error {
  constructor(readonly instructionCode: DocxRegionInstructionCode) {
    super(instructionCode);
    this.name = 'DocxRegionSelectionError';
  }
}

export function docxBlockRelativeRect(
  blockBounds: Pick<DOMRect, 'left' | 'top' | 'width' | 'height'>,
  start: { x: number; y: number },
  end: { x: number; y: number },
): NormalizedRect {
  if (
    ![
      blockBounds.left,
      blockBounds.top,
      blockBounds.width,
      blockBounds.height,
      start.x,
      start.y,
      end.x,
      end.y,
    ].every(Number.isFinite) ||
    blockBounds.width <= 0 ||
    blockBounds.height <= 0
  )
    throw new DocxRegionSelectionError('docx_region_unavailable');
  const left = Math.max(blockBounds.left, Math.min(start.x, end.x));
  const top = Math.max(blockBounds.top, Math.min(start.y, end.y));
  const right = Math.min(
    blockBounds.left + blockBounds.width,
    Math.max(start.x, end.x),
  );
  const bottom = Math.min(
    blockBounds.top + blockBounds.height,
    Math.max(start.y, end.y),
  );
  if (
    right - left < DOCX_REGION_MIN_CSS_PIXELS ||
    bottom - top < DOCX_REGION_MIN_CSS_PIXELS
  )
    throw new DocxRegionSelectionError('docx_region_too_small');
  return {
    x: stableRatio(left - blockBounds.left, blockBounds.width),
    y: stableRatio(top - blockBounds.top, blockBounds.height),
    width: stableRatio(right - left, blockBounds.width),
    height: stableRatio(bottom - top, blockBounds.height),
  };
}

export function docxRegionBlock(
  target: EventTarget | null,
  root: HTMLElement,
): HTMLElement | null {
  if (!(target instanceof Element)) return null;
  const block = target.closest<HTMLElement>('[data-block-id]');
  return block && root.contains(block) && block.dataset.blockId ? block : null;
}

/** Exact stable-ID lookup only. Duplicate IDs are ambiguous and stay unresolved. */
export function uniqueDocxRegionBlock(
  root: HTMLElement,
  blockId: string,
): HTMLElement | null {
  if (!blockId) return null;
  const selector = `[data-block-id="${CSS.escape(blockId)}"]`;
  const matches = root.querySelectorAll<HTMLElement>(selector);
  return matches.length === 1 ? matches[0]! : null;
}

const NORMALIZED_PRECISION = 1_000_000;
function stableRatio(value: number, total: number): number {
  return (
    Math.round((value / total) * NORMALIZED_PRECISION) / NORMALIZED_PRECISION
  );
}
