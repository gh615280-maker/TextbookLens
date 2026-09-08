import { describe, expect, it } from 'vitest';
import { localPdfOptions } from './pdf-options';

describe('local PDF resources', () => {
  it('uses bundled same-origin decoders and fonts without accepting a source URL', () => {
    const source = new Uint8Array([1, 2, 3]).buffer;
    const options = localPdfOptions(source);
    expect([...options.data]).toEqual([1, 2, 3]);
    for (const url of [
      options.wasmUrl,
      options.cMapUrl,
      options.standardFontDataUrl,
    ]) {
      expect(new URL(url).origin).toBe(window.location.origin);
      expect(new URL(url).pathname).toMatch(/^\/pdfjs\//);
    }
    expect(options).toMatchObject({
      isEvalSupported: false,
      stopAtErrors: true,
      cMapPacked: true,
    });
    expect(options).not.toHaveProperty('url');
  });
});
