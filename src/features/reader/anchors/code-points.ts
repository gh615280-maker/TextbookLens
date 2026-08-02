function boundaries(text: string): number[] {
  const result = [0];
  let utf16Offset = 0;
  for (const codePoint of Array.from(text)) {
    utf16Offset += codePoint.length;
    result.push(utf16Offset);
  }
  return result;
}

function requireIntegerInRange(offset: number, maximum: number, name: string): void {
  if (!Number.isInteger(offset) || offset < 0 || offset > maximum) {
    throw new RangeError(`${name} must be an integer boundary within the text`);
  }
}

export function toCodePointOffset(text: string, utf16Offset: number): number {
  const utf16Boundaries = boundaries(text);
  requireIntegerInRange(utf16Offset, text.length, 'UTF-16 offset');
  const codePointOffset = utf16Boundaries.indexOf(utf16Offset);
  if (codePointOffset < 0) {
    throw new RangeError('UTF-16 offset must not split a surrogate pair');
  }
  return codePointOffset;
}

export function toUtf16Offset(text: string, codePointOffset: number): number {
  const utf16Boundaries = boundaries(text);
  requireIntegerInRange(codePointOffset, utf16Boundaries.length - 1, 'Code-point offset');
  return utf16Boundaries[codePointOffset];
}
