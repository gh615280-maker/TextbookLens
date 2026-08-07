import { describe, expect, it } from 'vitest';
import {
  denormalizeRects,
  normalizeRects,
  selectionFromRange,
} from './pdf-selection';

describe('PDF selection geometry', () => {
  it('clips, orders and round-trips rectangles', () => {
    const bounds = { page: 1, left: 100, top: 50, width: 800, height: 1000 };
    const normalized = normalizeRects(bounds, [
      { left: 200, top: 150, width: 100, height: 30 },
      { left: 50, top: 40, width: 80, height: 30 },
      { left: 120, top: 80, width: 0, height: 4 },
    ]);
    expect(normalized).toEqual([
      { x: 0, y: 0, width: 0.0375, height: 0.02 },
      { x: 0.125, y: 0.1, width: 0.125, height: 0.03 },
    ]);
    expect(denormalizeRects(bounds, normalized)[1]).toMatchObject({
      left: 200,
      top: 150,
      width: 100,
      height: 30,
    });
  });

  it('copies adjacent-page selections into a normalized locator', () => {
    document.body.innerHTML =
      '<div data-page-number="1">first <span>page</span></div><div data-page-number="2">second page</div>';
    const pages = [
      ...document.querySelectorAll<HTMLElement>('[data-page-number]'),
    ];
    pages.forEach((page, index) =>
      Object.defineProperty(page, 'getBoundingClientRect', {
        value: () => new DOMRect(0, index * 100, 100, 100),
      }),
    );
    const range = document.createRange();
    range.setStart(pages[0].firstChild!, 0);
    range.setEnd(pages[1].firstChild!, 6);
    Object.defineProperty(range, 'getClientRects', {
      value: () => [
        { left: 2, top: 3, width: 10, height: 10 },
        { left: 2, top: 103, width: 12, height: 10 },
      ],
    });
    const selected = selectionFromRange(range, pages);
    expect(selected?.text).toBe('first pagesecond');
    expect(selected?.locator).toMatchObject({
      format: 'pdf',
      startPage: 1,
      endPage: 2,
    });
    expect(selected?.quote.exact).toBe('first pagesecond');
    expect(selected?.sectionId).toBeNull();
    expect(selected?.locator).toMatchObject({
      rectsByPage: {
        1: [{ x: 0.02, y: 0.03, width: 0.1, height: 0.1 }],
        2: [{ x: 0.02, y: 0.03, width: 0.12, height: 0.1 }],
      },
    });
  });
});
