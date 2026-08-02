import { describe, expect, it } from 'vitest';

import { normalizePdfPage } from './pdf-layout';

describe('normalizePdfPage', () => {
  it('merges a baseline left-to-right and preserves formulas', () => {
    const blocks = normalizePdfPage([
      item('2x + 1', 140, 700),
      item('y = ', 48, 700),
      item('Ignored trailing space ', 48, 650),
    ]);

    expect(blocks.map((block) => block.text)).toEqual([
      'y = 2x + 1',
      'Ignored trailing space',
    ]);
  });

  it('starts a paragraph after a large vertical gap in stable reading order', () => {
    const blocks = normalizePdfPage([
      item('second line', 48, 670),
      item('first line', 48, 700),
      item('next paragraph', 48, 590),
    ]);

    expect(blocks.map((block) => block.text)).toEqual([
      'first line second line',
      'next paragraph',
    ]);
  });
});

function item(text: string, x: number, y: number) {
  return { str: text, transform: [12, 0, 0, 12, x, y] as const, width: text.length * 6 };
}
