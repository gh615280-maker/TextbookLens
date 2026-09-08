# v0.2.0 release preparation

Scope approved in the conversation on 2026-09-08. Work in the current shared checkout; do not include unrelated experiments or user files. Build and test artifacts use a separate task directory. Do not tag, push, or publish as part of preparation.

- [x] Fix PDF decoding in `src/features/reader/pdf/PdfReaderAdapter.ts`, `src/features/import/parsers/pdf-parser.ts`, and `src/features/indexing/pdf-page-renderer.ts` using shared local PDF.js resources. Bundle decoder/font resources from the pinned dependency, retain script confinement, and surface page-render failures. Verify JBIG2 pages from the desktop test corpus and a synthetic browser regression.
- [x] Fix `SelectionMenu.tsx` positioning when its form grows; retain cancellation and focus behavior. Verify bottom-edge question/note forms in normal and narrow windows.
- [x] Restore library card/table layout, persistent reader navigation, and distinct resize/scroll hit areas in the library components, `ReaderLayout.tsx`, and reader styles. Verify long book titles and actual desktop interactions.
- [x] Set application, lockfile, and package versions to 0.2.0. Document Ollama text/vision scope; defer LM Studio real-machine validation to 0.2.1. Existing cloud indexing remains available; do not claim local whole-book OCR/indexing.
- [x] Run affected tests, full application regression, formatting, lint, bindings, dependency/license/fixture checks, and release artifact scans. Produce Release EXE/MSI from a controlled source snapshot, preserving existing migration bytes.
- [x] Exercise fresh Windows installation and previous-version upgrade; verify library/history/settings/profile retention and uninstall behavior. Recheck actual textbook image questions, oversized input, cancellation, stopped-service recovery, and offline startup where the environment permits. Record unexecuted gates explicitly.

Acceptance: the two observed P1 defects are fixed; the selected common UI defects are fixed; installable 0.2.0 artifacts and reproducible evidence are available. Remaining platform or model limitations are stated accurately. No automatic cloud fallback, model download, or whole-book local OCR is added.

Validation and remaining coverage limits: [v0.2.0-validation.md](../../releases/v0.2.0-validation.md). Completed preparation does not mean every platform/model scenario passed.
