import { describe, expect, it } from 'vitest';

import {
  pageAtPoint,
  pageRelativeRect,
  PdfRegionSelectionError,
  normalizeForPdfRotation,
} from './pdf-region-selection';

describe('PDF region selection geometry', () => {
  it('normalizes one page-relative rect independently of zoom and scroll', () => {
    expect(
      pageRelativeRect(
        3,
        { left: 100, top: 200, width: 400, height: 800 },
        { x: 140, y: 280 },
        { x: 300, y: 600 },
      ),
    ).toEqual({
      page: 3,
      rect: { x: 0.1, y: 0.1, width: 0.4, height: 0.4 },
    });
    expect(
      pageRelativeRect(
        3,
        { left: -200, top: -400, width: 800, height: 1600 },
        { x: -120, y: -240 },
        { x: 200, y: 400 },
      ).rect,
    ).toEqual({ x: 0.1, y: 0.1, width: 0.4, height: 0.4 });
  });

  it('rejects tiny rectangles with a stable instruction code', () => {
    expect(() =>
      pageRelativeRect(
        1,
        { left: 0, top: 0, width: 100, height: 100 },
        { x: 10, y: 10 },
        { x: 17, y: 40 },
      ),
    ).toThrow(
      expect.objectContaining({ instructionCode: 'pdf_region_too_small' }),
    );
  });

  it('finds only the actual page under a pointer', () => {
    const root = document.createElement('div');
    const page = document.createElement('div');
    page.dataset.pageNumber = '1';
    page.getBoundingClientRect = () =>
      ({
        left: 10,
        top: 20,
        right: 110,
        bottom: 220,
        width: 100,
        height: 200,
      }) as DOMRect;
    root.append(page);
    expect(pageAtPoint(root, 50, 50)).toBe(page);
    expect(pageAtPoint(root, 5, 50)).toBeUndefined();
    expect(new PdfRegionSelectionError('pdf_region_cross_page').message).toBe(
      'pdf_region_cross_page',
    );
  });

  it('maps rotated viewport rectangles into stable unrotated page space', () => {
    const displayed = { x: 0.1, y: 0.2, width: 0.3, height: 0.4 };
    expect(normalizeForPdfRotation(displayed, 90)).toEqual({
      x: 0.2,
      y: 0.6,
      width: 0.4,
      height: 0.3,
    });
    expect(normalizeForPdfRotation(displayed, 180)).toEqual({
      x: 0.6,
      y: 0.4,
      width: 0.3,
      height: 0.4,
    });
    expect(normalizeForPdfRotation(displayed, 270)).toEqual({
      x: 0.4,
      y: 0.1,
      width: 0.4,
      height: 0.3,
    });
  });
});
