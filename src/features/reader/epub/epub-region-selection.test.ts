import { describe, expect, it } from 'vitest';

import {
  epubElementRelativeRect,
  epubRegionContainer,
  EpubRegionSelectionError,
  iframeElementRelativeRect,
  iframePointToReader,
} from './epub-region-selection';

describe('EPUB region selection geometry', () => {
  it('keeps an element-relative rect stable across reflow, font scale and window movement', () => {
    expect(
      epubElementRelativeRect(
        { left: 40, top: 80, width: 400, height: 200 },
        { x: 80, y: 100 },
        { x: 240, y: 180 },
      ),
    ).toEqual({ x: 0.1, y: 0.1, width: 0.4, height: 0.4 });
    expect(
      epubElementRelativeRect(
        { left: -120, top: -240, width: 800, height: 400 },
        { x: -40, y: -200 },
        { x: 280, y: -40 },
      ),
    ).toEqual({ x: 0.1, y: 0.1, width: 0.4, height: 0.4 });
    expect(
      iframeElementRelativeRect(
        { left: 500, top: 300 },
        { left: 20, top: 40, width: 200, height: 100 },
        { x: 540, y: 350 },
        { x: 620, y: 390 },
      ),
    ).toEqual({ x: 0.1, y: 0.1, width: 0.4, height: 0.4 });
  });

  it('maps iframe-local preview points into reader coordinates without persisting them', () => {
    expect(
      iframePointToReader(
        { left: 100, top: 50 },
        { left: 250, top: 180 },
        { x: 20, y: 30 },
      ),
    ).toEqual({ x: 170, y: 160 });
  });

  it('rejects tiny rectangles and only chooses a conservative block container', () => {
    expect(() =>
      epubElementRelativeRect(
        { left: 0, top: 0, width: 100, height: 100 },
        { x: 10, y: 10 },
        { x: 17, y: 40 },
      ),
    ).toThrow(
      expect.objectContaining({ instructionCode: 'epub_region_too_small' }),
    );
    const document = new DOMParser().parseFromString(
      '<body><section><p><span>target</span></p></section></body>',
      'text/html',
    );
    expect(
      epubRegionContainer(document.querySelector('span'), document)?.tagName,
    ).toBe('P');
    expect(
      epubRegionContainer(document.querySelector('section'), document),
    ).toBeNull();
    expect(new EpubRegionSelectionError('epub_region_cancelled').message).toBe(
      'epub_region_cancelled',
    );
  });
});
