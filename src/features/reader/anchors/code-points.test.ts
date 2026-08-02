import { describe, expect, it } from 'vitest';

import { toCodePointOffset, toUtf16Offset } from './code-points';

describe('code point offsets', () => {
  const text = 'A😀中B';

  it('converts UTF-16 offsets without splitting surrogate pairs', () => {
    expect(toCodePointOffset(text, 3)).toBe(2);
    expect(toUtf16Offset(text, 2)).toBe(3);
  });

  it('rejects invalid and surrogate-pair interior offsets', () => {
    expect(() => toCodePointOffset(text, -1)).toThrow();
    expect(() => toCodePointOffset(text, 2)).toThrow();
    expect(() => toCodePointOffset(text, 6)).toThrow();
    expect(() => toUtf16Offset(text, -1)).toThrow();
    expect(() => toUtf16Offset(text, 5)).toThrow();
  });

  it('round-trips every code point boundary', () => {
    for (let offset = 0; offset <= Array.from(text).length; offset += 1) {
      expect(toCodePointOffset(text, toUtf16Offset(text, offset))).toBe(offset);
    }
  });
});
