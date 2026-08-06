import type {
  DocumentLocator,
  NormalizedRect,
  RegionAnchor,
  TextQuote,
} from '../../../lib/generated/document';
import { findTextQuote } from '../anchors/text-quote';
import { uniqueDocxRegionBlock } from './docx-region-selection';
import { rangeFromDocxLocator } from './docx-selection';

export function addDocxRangeOverlay(
  root: HTMLElement,
  range: Range,
): () => void {
  const overlay = Object.assign(document.createElement('div'), {
    className: 'docx-marker-overlay',
  });
  overlay.setAttribute('aria-hidden', 'true');
  for (const rect of range.getClientRects()) {
    const marker = document.createElement('span');
    Object.assign(marker.style, {
      left: `${rect.left - root.getBoundingClientRect().left}px`,
      top: `${rect.top - root.getBoundingClientRect().top}px`,
      width: `${rect.width}px`,
      height: `${rect.height}px`,
    });
    overlay.append(marker);
  }
  root.append(overlay);
  return () => overlay.remove();
}

export function recoverDocxRange(
  root: HTMLElement,
  sectionId: string,
  quote: TextQuote,
  locator?: Extract<DocumentLocator, { format: 'docx' }>,
): Range | null {
  const sectionBlocks = [
    ...root.querySelectorAll<HTMLElement>('[data-block-id]'),
  ].filter((block) => block.dataset.sectionId === sectionId);
  const blocks = locator
    ? (() => {
        const startIndex = sectionBlocks.findIndex(
          (block) => block.dataset.blockId === locator.startBlockId,
        );
        const endIndex = sectionBlocks.findIndex(
          (block) => block.dataset.blockId === locator.endBlockId,
        );
        return startIndex < 0 || endIndex < startIndex
          ? []
          : sectionBlocks.slice(startIndex, endIndex + 1);
      })()
    : sectionBlocks;
  const text = blocks.map((block) => block.textContent ?? '').join('');
  const match = findTextQuote(text, quote);
  if (!match) return null;
  let startOffset = match.startCp;
  let startBlock: HTMLElement | undefined;
  for (const block of blocks) {
    const size = [...(block.textContent ?? '')].length;
    if (startOffset <= size) {
      startBlock = block;
      break;
    }
    startOffset -= size;
  }
  let endOffset = match.endCp;
  let endBlock: HTMLElement | undefined;
  for (const block of blocks) {
    const size = [...(block.textContent ?? '')].length;
    if (endOffset <= size) {
      endBlock = block;
      break;
    }
    endOffset -= size;
  }
  if (!startBlock || !endBlock) return null;
  return rangeFromDocxLocator(root, {
    format: 'docx',
    startBlockId: startBlock.dataset.blockId!,
    startOffset,
    endBlockId: endBlock.dataset.blockId!,
    endOffset,
  });
}

export function recoverDocxRegionRange(
  root: HTMLElement,
  anchor: RegionAnchor,
): Range | null {
  const locator = anchor.locator;
  if (
    locator.format !== 'docx' ||
    !/^[0-9a-f]{64}$/u.test(anchor.contentSha256) ||
    !anchor.textFallback ||
    !validRect(anchor.rect)
  )
    return null;
  const block = uniqueDocxRegionBlock(root, locator.blockId);
  const sectionId = block?.dataset.sectionId;
  if (!block || !sectionId) return null;
  const length = [...(block.textContent ?? '')].length;
  return recoverDocxRange(root, sectionId, anchor.textFallback, {
    format: 'docx',
    startBlockId: locator.blockId,
    startOffset: 0,
    endBlockId: locator.blockId,
    endOffset: length,
  });
}

export function addDocxRegionOverlay(
  root: HTMLElement,
  block: HTMLElement,
  rect: NormalizedRect,
): () => void {
  const overlay = Object.assign(document.createElement('div'), {
    className: 'docx-marker-overlay docx-region-marker-overlay',
  });
  const rootBounds = root.getBoundingClientRect();
  const bounds = block.getBoundingClientRect();
  const marker = document.createElement('span');
  marker.className = 'docx-marker-region';
  Object.assign(marker.style, {
    left: `${bounds.left - rootBounds.left + rect.x * bounds.width}px`,
    top: `${bounds.top - rootBounds.top + rect.y * bounds.height}px`,
    width: `${rect.width * bounds.width}px`,
    height: `${rect.height * bounds.height}px`,
  });
  overlay.append(marker);
  root.append(overlay);
  return () => overlay.remove();
}

function validRect(rect: NormalizedRect): boolean {
  return (
    Number.isFinite(rect.x) &&
    Number.isFinite(rect.y) &&
    Number.isFinite(rect.width) &&
    Number.isFinite(rect.height) &&
    rect.x >= 0 &&
    rect.y >= 0 &&
    rect.width > 0 &&
    rect.height > 0 &&
    rect.x + rect.width <= 1 &&
    rect.y + rect.height <= 1
  );
}
