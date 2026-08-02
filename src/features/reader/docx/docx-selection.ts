import type {
  DocumentLocator,
  TextQuote,
} from '../../../lib/generated/document';
import { toCodePointOffset, toUtf16Offset } from '../anchors/code-points';
import { createTextQuote } from '../anchors/text-quote';

export interface DocxRangeSnapshot {
  locator: DocumentLocator;
  text: string;
  quote: TextQuote;
  sectionId: string;
}

export function snapshotDocxRange(
  range: Range,
  root: HTMLElement,
): DocxRangeSnapshot | null {
  const start = endpoint(range.startContainer, range.startOffset, root);
  const end = endpoint(range.endContainer, range.endOffset, root);
  const text = range.toString();
  if (!start || !end || !text) return null;
  const first = compareEndpoints(start, end) <= 0 ? start : end;
  const last = first === start ? end : start;
  if (first.sectionId !== last.sectionId) return null;
  const section =
    root.querySelector<HTMLElement>(
      `[data-section-id="${CSS.escape(first.sectionId)}"]`,
    )?.parentElement?.textContent ??
    root.textContent ??
    '';
  const quoteStart = Math.max(0, [...section].join('').indexOf(text));
  return {
    text,
    sectionId: first.sectionId,
    locator: {
      format: 'docx',
      startBlockId: first.blockId,
      startOffset: first.offset,
      endBlockId: last.blockId,
      endOffset: last.offset,
    },
    quote: createTextQuote(
      section,
      [...section.slice(0, quoteStart)].length,
      [...section.slice(0, quoteStart + text.length)].length,
    ),
  };
}

export function rangeFromDocxLocator(
  root: HTMLElement,
  locator: Extract<DocumentLocator, { format: 'docx' }>,
): Range | null {
  const startBlock = root.querySelector<HTMLElement>(
    `[data-block-id="${CSS.escape(locator.startBlockId)}"]`,
  );
  const endBlock = root.querySelector<HTMLElement>(
    `[data-block-id="${CSS.escape(locator.endBlockId)}"]`,
  );
  if (!startBlock || !endBlock) return null;
  const start = textPoint(startBlock, locator.startOffset);
  const end = textPoint(endBlock, locator.endOffset);
  if (!start || !end) return null;
  const range = document.createRange();
  range.setStart(start.node, start.offset);
  range.setEnd(end.node, end.offset);
  return range;
}

function endpoint(
  node: Node,
  offset: number,
  root: HTMLElement,
): {
  blockId: string;
  sectionId: string;
  offset: number;
  ordinal: number;
} | null {
  const element =
    node.nodeType === Node.ELEMENT_NODE
      ? (node as Element)
      : node.parentElement;
  const block = element?.closest<HTMLElement>('[data-block-id]');
  const sectionId = block?.dataset.sectionId;
  const blockId = block?.dataset.blockId;
  if (!block || !sectionId || !blockId || !root.contains(block)) return null;
  const range = document.createRange();
  range.selectNodeContents(block);
  range.setEnd(node, offset);
  return {
    blockId,
    sectionId,
    offset: [...range.toString()].length,
    ordinal: [...root.querySelectorAll<HTMLElement>('[data-block-id]')].indexOf(
      block,
    ),
  };
}
function textPoint(
  block: HTMLElement,
  codePoints: number,
): { node: Text; offset: number } | null {
  const walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT);
  let remaining = codePoints;
  let node = walker.nextNode() as Text | null;
  while (node) {
    const count = [...node.data].length;
    if (remaining <= count)
      return { node, offset: toUtf16Offset(node.data, remaining) };
    remaining -= count;
    node = walker.nextNode() as Text | null;
  }
  return remaining === 0
    ? { node: document.createTextNode(''), offset: 0 }
    : null;
}
function compareEndpoints(
  a: { ordinal: number; offset: number },
  b: { ordinal: number; offset: number },
): number {
  return a.ordinal - b.ordinal || a.offset - b.offset;
}
export { toCodePointOffset };
