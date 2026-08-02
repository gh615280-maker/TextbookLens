import type { DocumentLocator, NormalizedRect, TextQuote } from '../../../lib/generated/document';
import { createTextQuote } from '../anchors/text-quote';

export interface ClientRectLike { left: number; top: number; width: number; height: number; }
export interface PageBounds extends ClientRectLike { page: number; }

export function normalizeRects(bounds: PageBounds, rects: Iterable<ClientRectLike>): NormalizedRect[] {
  const result: NormalizedRect[] = [];
  for (const rect of rects) {
    const left = Math.max(bounds.left, rect.left);
    const top = Math.max(bounds.top, rect.top);
    const right = Math.min(bounds.left + bounds.width, rect.left + rect.width);
    const bottom = Math.min(bounds.top + bounds.height, rect.top + rect.height);
    if (right <= left || bottom <= top) continue;
    result.push({ x: (left - bounds.left) / bounds.width, y: (top - bounds.top) / bounds.height, width: (right - left) / bounds.width, height: (bottom - top) / bounds.height });
  }
  return result.sort((a, b) => a.y - b.y || a.x - b.x);
}

export function denormalizeRects(bounds: PageBounds, rects: readonly NormalizedRect[]): ClientRectLike[] {
  return rects.map((rect) => ({ left: bounds.left + rect.x * bounds.width, top: bounds.top + rect.y * bounds.height, width: rect.width * bounds.width, height: rect.height * bounds.height }));
}

export interface PdfSelection { locator: DocumentLocator; quote: TextQuote; sectionId: string; text: string; }

export function selectionFromRange(range: Range, pages: readonly HTMLElement[]): PdfSelection | null {
  const startPage = containingPage(range.startContainer, pages);
  const endPage = containingPage(range.endContainer, pages);
  if (!startPage || !endPage || endPage.page < startPage.page || endPage.page - startPage.page > 1) return null;
  const text = range.toString();
  if (!text) return null;
  const rectsByPage: Record<number, NormalizedRect[]> = {};
  for (const page of pages) {
    const pageNumber = Number(page.dataset.pageNumber);
    if (pageNumber < startPage.page || pageNumber > endPage.page) continue;
    const pageRect = page.getBoundingClientRect();
    const normalized = normalizeRects({ page: pageNumber, ...pageRect }, range.getClientRects());
    if (normalized.length) rectsByPage[pageNumber] = normalized;
  }
  return { text, sectionId: `page-${startPage.page}`, locator: { format: 'pdf', startPage: startPage.page, endPage: endPage.page, rectsByPage }, quote: createTextQuote(text, 0, [...text].length) };
}

function containingPage(node: Node, pages: readonly HTMLElement[]): { page: number } | null {
  const element = node.nodeType === Node.ELEMENT_NODE ? node as Element : node.parentElement;
  const page = element?.closest<HTMLElement>('[data-page-number]');
  if (!page || !pages.includes(page)) return null;
  const number = Number(page.dataset.pageNumber);
  return Number.isInteger(number) && number > 0 ? { page: number } : null;
}
