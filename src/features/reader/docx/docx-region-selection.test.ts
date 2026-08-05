import { describe, expect, it } from 'vitest';

import {
  docxBlockRelativeRect,
  docxRegionBlock,
  DocxRegionSelectionError,
  uniqueDocxRegionBlock,
} from './docx-region-selection';

describe('DOCX region selection geometry', () => {
  it('keeps a block-relative rect stable across rerender and resize', () => {
    expect(
      docxBlockRelativeRect(
        { left: 50, top: 100, width: 500, height: 200 },
        { x: 100, y: 120 },
        { x: 300, y: 200 },
      ),
    ).toEqual({ x: 0.1, y: 0.1, width: 0.4, height: 0.4 });
    expect(
      docxBlockRelativeRect(
        { left: -200, top: -300, width: 1000, height: 400 },
        { x: -100, y: -260 },
        { x: 300, y: -100 },
      ),
    ).toEqual({ x: 0.1, y: 0.1, width: 0.4, height: 0.4 });
  });

  it('rejects tiny rectangles and cross-block ownership stays explicit', () => {
    expect(() =>
      docxBlockRelativeRect(
        { left: 0, top: 0, width: 100, height: 100 },
        { x: 10, y: 10 },
        { x: 40, y: 17 },
      ),
    ).toThrow(
      expect.objectContaining({ instructionCode: 'docx_region_too_small' }),
    );
    const root = document.createElement('main');
    root.innerHTML =
      '<p data-block-id="a"><span>one</span></p><p data-block-id="b">two</p>';
    expect(
      docxRegionBlock(root.querySelector('span'), root)?.dataset.blockId,
    ).toBe('a');
    expect(
      new DocxRegionSelectionError('docx_region_cross_block').message,
    ).toBe('docx_region_cross_block');
  });

  it('resolves exactly one stable block and rejects duplicate IDs as ambiguous', () => {
    const root = document.createElement('main');
    root.innerHTML = '<p data-block-id="stable">one</p>';
    expect(uniqueDocxRegionBlock(root, 'stable')).toBe(root.firstElementChild);
    root.insertAdjacentHTML(
      'beforeend',
      '<p data-block-id="stable">duplicate</p>',
    );
    expect(uniqueDocxRegionBlock(root, 'stable')).toBeNull();
    expect(uniqueDocxRegionBlock(root, 'missing')).toBeNull();
  });
});
