# ADR 0003: Format adapters stop at normalized blocks and anchors

## Status

Accepted for Phase 2 import.

## Decision

The import pipeline uses the vendor-supported parser for each format, retains the
source file for its Phase 3 renderer, and persists only normalized sections,
blocks, and format-specific anchors. It does not attempt to make PDF pages,
EPUB XHTML, and DOCX layout share a renderer.

- PDF.js 6.2.108 uses `getDocument({ data })`, `getMetadata()`, `getPage()`,
  `getTextContent()`, `PDFDocumentProxy.cleanup()`, and loading-task
  `destroy()`. Its worker source is a Vite URL for `pdf.worker.mjs`.
- EPUB.js 0.3.93 opens an `ArrayBuffer` with replacements disabled, awaits
  `book.ready`, iterates `book.spine.each()`, loads each section with
  `section.load(book.load.bind(book))`, creates anchors with
  `section.cfiFromElement()`, and releases each item with `section.unload()`
  before `book.destroy()`.
- The committed EPUB fixture proves a CFI made by `cfiFromElement()` resolves
  through `book.getRange()` into the same spine section. That proof is about
  structure and anchors, not a common EPUB renderer.
- Mammoth 1.12.0 will use `convertToHtml({ arrayBuffer }, { styleMap })`; its
  generated HTML remains untrusted until Task 6's DOMPurify allowlist removes
  executable and external content before it is written as derived HTML.

## Consequences

PDF imports have one-based page locators without selection rectangles. EPUB
imports retain CFIs and section IDs. DOCX imports use code-point offsets over
sanitized derived HTML. No adapter performs DRM decryption, OCR, or image-text
guessing; encrypted, corrupt, and textless sources report normalized errors.

The memory lifecycle is bounded: PDF document/loading resources are released in
`finally`; EPUB sections unload as they are processed and the book is destroyed
in `finally`; Mammoth conversion bytes and DOMs are discarded after sanitized
HTML and blocks are emitted.
