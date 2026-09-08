/** Only document bytes and bundled same-origin resources reach PDF.js. */
export function localPdfOptions(source: ArrayBuffer) {
  const root = new URL(import.meta.env.BASE_URL, window.location.href);
  return {
    data: new Uint8Array(source),
    isEvalSupported: false,
    stopAtErrors: true,
    wasmUrl: new URL('pdfjs/wasm/', root).href,
    cMapUrl: new URL('pdfjs/cmaps/', root).href,
    cMapPacked: true,
    standardFontDataUrl: new URL('pdfjs/standard_fonts/', root).href,
  };
}
